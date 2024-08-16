use allocator_api2::alloc::AllocError;
use allocator_api2::alloc::Allocator;
use allocator_api2::vec::Vec;
use std::cell::Cell;
use std::cell::RefCell;
use std::cell::UnsafeCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::mem::MaybeUninit;
use std::{alloc, ptr::NonNull};

pub use winwin_common::{Key, KeyState};

mod events;
pub use events::*;

mod wm;
pub use wm::*;

#[macro_export]
macro_rules! error_if_err {
    ($result:expr) => {
        if let Err(e) = $result {
            tracing::error!(error = ?e);
        }
    };
}

#[macro_export]
macro_rules! error_if {
    ($failed:expr) => {
        if $failed.as_bool() {
            let e = windows::core::Error::from_win32();
            tracing::error!(error = ?e);
        }
    };
}

pub struct Context {
    // Arena allocator for temporary (on event) allocations.
    arena: Arena,
    pub cache: Cache,
}

impl Context {
    pub(crate) fn new() -> Self {
        let arena = Arena::new_with_global_alloc();
        let monitors = wm::get_all_monitors_live(&arena);

        let mut map = HashMap::with_capacity(monitors.len());
        let windows = wm::get_all_windows_live(&arena);

        for monitor in monitors {
            let windows_on_monitor = windows.iter().filter(|w| w.is_on_monitor(monitor));
            let vd = VecDeque::from_iter(windows_on_monitor);
            map.insert(monitor, vd);
        }

        Self {
            arena,
            cache: Cache::new(map),
        }
    }
}

pub struct Arena {
    mem: NonNull<u8>,
    end: Cell<usize>,
    used: Cell<usize>,
    capacity: usize,
}

impl Arena {
    pub fn new_with_global_alloc() -> Self {
        // Reserve 4GB, commit as needed.
        let size = u32::MAX as usize;
        let layout = alloc::Layout::array::<u8>(size).expect("arguments are correct");
        let mem = unsafe { alloc::alloc(layout) };
        let mem = NonNull::new(mem).expect("global alloc should not fail");

        Arena {
            mem,
            end: Cell::new(0),
            used: Cell::new(0),
            capacity: size,
        }
    }

    pub fn reset(&self) {
        self.end.set(0);
        self.used.set(0);
    }

    pub fn slice_uninit<'a, T: Sized>(&'a self, size: usize) -> &'a [MaybeUninit<T>] {
        let layout = alloc::Layout::array::<T>(size).unwrap();
        let ptr = self.allocate(layout).unwrap();
        let s = unsafe { std::slice::from_raw_parts(ptr.cast().as_ptr(), size) };
        s
    }

    pub fn slice_mut_uninit<'a, T: Sized>(&'a self, size: usize) -> &'a mut [MaybeUninit<T>] {
        let layout = alloc::Layout::array::<T>(size).unwrap();
        let ptr = self.allocate(layout).unwrap();
        let s = unsafe { std::slice::from_raw_parts_mut(ptr.cast().as_ptr(), size) };
        s
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        let layout = alloc::Layout::array::<u8>(self.capacity).expect("arguments are correct");
        unsafe { alloc::dealloc(self.mem.as_ptr(), layout) };
    }
}

unsafe impl Allocator for &Arena {
    fn allocate(&self, layout: alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        unsafe {
            let end = self.end.get();
            let curr_ptr = self.mem.as_ptr().add(end);
            let size = layout.size();
            let align = layout.align();

            let offset = curr_ptr.align_offset(align);
            if offset == usize::MAX || end + offset + size > self.capacity {
                return Err(AllocError);
            }

            let aligned_ptr = curr_ptr.add(offset);
            self.end.set(end + offset + size);
            self.used.set(self.used.get() + size);

            Ok(NonNull::slice_from_raw_parts(
                NonNull::new_unchecked(aligned_ptr),
                size,
            ))
        }
    }

    unsafe fn deallocate(&self, _ptr: NonNull<u8>, layout: alloc::Layout) {
        // Once all allocations are freed we reset this arena.
        let size = layout.size();
        self.used.set(self.used.get() - size);

        if self.used.get() == 0 {
            self.end.set(0);
        }
    }
}

pub trait FromIteratorWithAlloc<T, A: Allocator>: Sized {
    fn from_iter_with_alloc<I: IntoIterator<Item = T>>(iter: I, alloc: A) -> Self;
}

impl<T, A: Allocator> FromIteratorWithAlloc<T, A> for Vec<T, A> {
    fn from_iter_with_alloc<I: IntoIterator<Item = T>>(iter: I, alloc: A) -> Self {
        let iter = iter.into_iter();
        let mut my_vec = Vec::with_capacity_in(iter.size_hint().0, alloc);

        for item in iter {
            my_vec.push(item);
        }

        my_vec
    }
}

pub trait IteratorCollectWithAlloc: Iterator {
    fn collect_with<T, A, C>(self, alloc: A) -> C
    where
        Self: Sized + IntoIterator<Item = T>,
        C: FromIteratorWithAlloc<T, A>,
        A: Allocator,
    {
        C::from_iter_with_alloc(self, alloc)
    }
}

impl<I: Iterator> IteratorCollectWithAlloc for I {}

#[derive(Default)]
pub struct Cache {
    monitor_layouts: RefCell<HashMap<Monitor, Layout>>,
    window_queues: UnsafeCell<HashMap<Monitor, VecDeque<Window>>>,
}

impl Cache {
    pub fn new(precache: HashMap<Monitor, VecDeque<Window>>) -> Self {
        Self {
            monitor_layouts: RefCell::default(),
            window_queues: UnsafeCell::new(precache),
        }
    }

    pub fn remember_layout(&self, monitor: Monitor, layout: Layout) {
        let mut monitor_layouts = self.monitor_layouts.borrow_mut();
        monitor_layouts.insert(monitor, layout);
    }

    pub fn layout_on(&self, monitor: Monitor) -> Layout {
        let monitor_layouts = self.monitor_layouts.borrow();
        *monitor_layouts.get(&monitor).unwrap_or(&Layout::None)
    }

    pub(crate) fn update_queue(&self, monitor: Monitor, window: Window) {
        // SAFETY: There are no outstanding references to inner data because we do not hand out any
        // and do not retain any.
        let queues = unsafe { &mut *self.window_queues.get() };

        let entry = queues.entry(monitor);
        match entry {
            Entry::Occupied(_) => {}
            Entry::Vacant(e) => {
                // There was no cache entry for this monitor so we only need to create one.
                e.insert(VecDeque::from([window]));
                return;
            }
        };

        // There are three cases:
        // 1. Window was not present in any queue and must be added.
        // 2. Window was in different queue and must be moved.
        // 3. Windows was in correct queue but must be moved to the front.
        // Looping unconditionaly saves us from figuring out which case we were in. Instead we just
        // add to correct queue once we know that window does not exist elsewhere.
        for q in queues.values_mut() {
            q.retain(|w| *w != window);
        }
        let queue = queues
            .get_mut(&monitor)
            .expect("if we got here queue must exist");
        queue.push_front(window);
        dbg!(&queues);
    }

    pub(crate) fn monitor_with_window(&self, window: Window) -> Monitor {
        // SAFETY: There are no outstanding references to inner data because we do not hand out any
        // and do not retain any.
        let queues = unsafe { &*self.window_queues.get() };
        for (k, q) in queues.iter() {
            if q.contains(&window) {
                return *k;
            }
        }

        Monitor::default()
    }

    pub(crate) fn windows_on_monitor<'a>(
        &self,
        ctx: &'a Context,
        monitor: Monitor,
    ) -> Vec<Window, &'a Arena> {
        // SAFETY: There are no outstanding references to inner data because we do not hand out any
        // and do not retain any.
        let queues = unsafe { &*self.window_queues.get() };
        queues
            .get(&monitor)
            .unwrap_or(&VecDeque::new())
            .iter()
            .copied()
            .collect_with(&ctx.arena)
    }

    pub(crate) fn monitors<'a>(&self, ctx: &'a Context) -> Vec<Monitor, &'a Arena> {
        // SAFETY: There are no outstanding references to inner data because we do not hand out any
        // and do not retain any.
        let queues = unsafe { &*self.window_queues.get() };
        queues.keys().copied().collect_with(&ctx.arena)
    }

    pub(crate) fn windows<'a>(&self, ctx: &'a Context) -> Vec<Window, &'a Arena> {
        // SAFETY: There are no outstanding references to inner data because we do not hand out any
        // and do not retain any.
        let queues = unsafe { &*self.window_queues.get() };
        queues
            .values()
            .map(|q| q.iter())
            .flatten()
            .copied()
            .collect_with(&ctx.arena)
    }

    pub(crate) fn focused_monitor(&self) -> Monitor {
        todo!()
    }

    pub(crate) fn focused_window(&self) -> Window {
        todo!()
    }
}
