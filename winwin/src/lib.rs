use allocator_api2::alloc::AllocError;
use allocator_api2::alloc::Allocator;
use allocator_api2::alloc::Global as GlobalAllocator;
use allocator_api2::vec::Vec;
use std::cell::Cell;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::sync::mpsc::SyncSender;
use std::{alloc, ptr::NonNull};

pub use winwin_common::{Key, KeyState};

mod events;
pub use events::*;

mod wm;
pub use wm::*;

#[macro_export]
macro_rules! trace_result {
    ($($result:expr),* $(,)?) => {
        $(
            if let Err(e) = $result {
                tracing::error!(error = ?e);
            }
        )*
    };
}

#[macro_export]
macro_rules! trace_result_b {
    ($($success:expr),* $(,)?) => {
        $(
            if !$success.as_bool() {
                let e = windows::core::Error::from_win32();
                tracing::error!(error = ?e);
            }
        )*
    };
}

pub struct Context<A: Allocator = GlobalAllocator> {
    arena: Arena,
    alloc: A,
    cache: Cache,

    // Make `Context` !Send and !Sync.
    _marker: PhantomData<*mut ()>,
}

impl Context<GlobalAllocator> {
    pub fn new() -> Self {
        let arena = Arena::new_with_global_alloc();
        let alloc = GlobalAllocator;
        let cache = Cache::default();

        Self {
            arena,
            alloc,
            cache: cache,

            _marker: PhantomData,
        }
    }
}

impl<A: Allocator> Context<A> {
    pub fn new_in(a: A) -> Self {
        let arena = Arena::new_with_global_alloc();
        let cache = Cache::default();

        Self {
            arena,
            alloc: a,
            cache: cache,

            _marker: PhantomData,
        }
    }
}

impl<A: Allocator + Copy> Context<A> {}

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

// TODO: Add Allocator bound to cache containers once it stabilizes.
#[derive(Default)]
pub struct Cache {
    inner: UnsafeCell<InnerCache>,
}

#[derive(Default)]
struct InnerCache {
    key_map: KeyMap,
    monitor_layouts: HashMap<Monitor, Layout>,
    window_queues: VecDeque<(Monitor, VecDeque<Window>)>,
}

impl Cache {
    pub(crate) fn save_layout(&self, monitor: Monitor, layout: Layout) {
        // SAFETY: We do not create nor retain any references to cache data, everything is copied
        // out of the cache.
        let cache = unsafe { &mut *self.inner.get() };
        cache.monitor_layouts.insert(monitor, layout);
    }

    pub(crate) fn layout_on(&self, monitor: Monitor) -> Layout {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &mut *self.inner.get() };
        *cache.monitor_layouts.get(&monitor).unwrap_or(&Layout::None)
    }

    pub(crate) fn update_window_queue(&self, monitor: Monitor, window: Window) {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &mut *self.inner.get() };
        let queues = &mut cache.window_queues;

        // There are three cases:
        // 1. Window was not present in any queue and must be added.
        // 2. Window was in different queue and must be moved.
        // 3. Windows was in correct queue but must be moved to the front.
        // Looping unconditionaly saves us from figuring out which case we were in. We simply try
        // to remove from every queue than add it to correct slot.
        for (_, q) in queues.iter_mut() {
            q.retain(|w| *w != window);
        }
        let target_queue_idx = queues
            .iter()
            .position(|(m, _)| *m == monitor)
            .expect("monitor must have its queue");
        let queue = &mut queues[target_queue_idx].1;
        queue.push_front(window);

        // We update monitor ordering because moved windows must have been focused.
        // Removing and insterting seems to be cheap for VecDeque.
        let queue = queues
            .remove(target_queue_idx)
            .expect("monitor must have its queue");
        queues.push_front(queue);
    }

    pub(crate) fn add_window_queue(&self, monitor: Monitor) {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &mut *self.inner.get() };
        cache.window_queues.push_back((monitor, VecDeque::new()));
    }

    pub(crate) fn add_window_to_queue(&self, window: Window, monitor: Monitor) {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &mut *self.inner.get() };
        let queue = &mut cache
            .window_queues
            .iter_mut()
            .find(|(m, _)| *m == monitor)
            .expect("monitor must have its queue")
            .1;
        queue.push_front(window);
    }

    pub(crate) fn remove_window_from_queue(&self, window: Window, monitor: Monitor) {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &mut *self.inner.get() };
        let queue = &mut cache
            .window_queues
            .iter_mut()
            .find(|(m, _)| *m == monitor)
            .expect("monitor must have its queue")
            .1;
        queue.retain(|w| *w != window);
    }

    pub(crate) fn update_input<A>(
        &self,
        ctx: &Context<A>,
        kb_delta: KBDelta,
        command_tx: SyncSender<KeyboardOp>,
    ) -> Input<A>
    where
        A: Allocator + Copy,
    {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &mut *self.inner.get() };
        cache.key_map.update(kb_delta);
        let input = cache.key_map.input(ctx, command_tx);
        input
    }

    pub(crate) fn fill<A>(&self, ctx: &Context<A>)
    where
        A: Allocator + Copy,
    {
        let monitors = get_monitors_live(ctx);
        let windows = get_windows_live(ctx);
        let mut window_queues = VecDeque::new();
        for monitor in monitors {
            let queue = windows
                .iter()
                .copied()
                .filter(|w| w.is_on_monitor(monitor))
                .collect();

            window_queues.push_back((monitor, queue));
        }

        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &mut *self.inner.get() };
        cache.window_queues = window_queues;
    }

    pub(crate) fn monitor_with_window(&self, window: Window) -> Monitor {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &mut *self.inner.get() };
        let queues = &cache.window_queues;

        for (k, q) in queues.iter() {
            if q.contains(&window) {
                return *k;
            }
        }

        Monitor::default()
    }

    pub(crate) fn windows_on_monitor<A>(&self, ctx: &Context<A>, monitor: Monitor) -> Vec<Window, A>
    where
        A: Allocator + Copy,
    {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &*self.inner.get() };
        let queues = &cache.window_queues;
        queues
            .iter()
            .find(|(m, _)| *m == monitor)
            .unwrap_or(queues.front().unwrap())
            .1
            .iter()
            .copied()
            .collect_with(ctx.alloc)
    }

    pub(crate) fn monitors<A>(&self, ctx: &Context<A>) -> Vec<Monitor, A>
    where
        A: Allocator + Copy,
    {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &*self.inner.get() };
        let queues = &cache.window_queues;
        queues
            .iter()
            .map(|(m, _)| m)
            .copied()
            .collect_with(ctx.alloc)
    }

    pub(crate) fn windows<A>(&self, ctx: &Context<A>) -> Vec<Window, A>
    where
        A: Allocator + Copy,
    {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &*self.inner.get() };
        let queues = &cache.window_queues;
        queues
            .iter()
            .map(|(_, q)| q.iter())
            .flatten()
            .copied()
            .collect_with(ctx.alloc)
    }

    pub(crate) fn focused_monitor(&self) -> Monitor {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &*self.inner.get() };
        cache
            .window_queues
            .front()
            .expect("there is at least one monitor")
            .0
    }

    pub(crate) fn focused_window(&self) -> Window {
        // SAFETY: See safety section for `save_layout`.
        let cache = unsafe { &*self.inner.get() };
        *cache
            .window_queues
            .front()
            .unwrap()
            .1
            .front()
            .unwrap_or(&Window::default())
    }
}

#[derive(Default)]
pub struct KeyMap {
    keys: [u32; 8],
}

impl KeyMap {
    pub fn update(&mut self, kb_delta: KBDelta) {
        let idx = (kb_delta.vk_code / 32) as usize;
        let bit = kb_delta.vk_code % 32;
        match kb_delta.key_state {
            KeyState::Up => {
                self.keys[idx] &= !(1 << bit);
            }
            KeyState::Down => {
                self.keys[idx] |= 1 << bit;
            }
        }
    }

    pub fn input<'a, A>(&self, ctx: &'a Context<A>, tx: SyncSender<KeyboardOp>) -> Input<A>
    where
        A: Allocator + Copy,
    {
        let mut pressed_keys = Vec::new_in(ctx.alloc);

        for i in 0..256 {
            let idx = i / 32;
            let bit = i % 32;
            if self.keys[idx] & (1 << bit) != 0 {
                pressed_keys.push(Key::from_vk_code(i as u8));
            }
        }

        Input {
            keys: pressed_keys,
            intercept_tx: tx,
        }
    }
}

#[derive(Debug)]
pub struct Input<A: Allocator> {
    keys: Vec<Key, A>,
    intercept_tx: SyncSender<KeyboardOp>,
}

impl<A: Allocator> Drop for Input<A> {
    fn drop(&mut self) {
        let _ = self.intercept_tx.try_send(KeyboardOp::DoNothing);
    }
}

impl<A: Allocator> Input<A> {
    pub fn pressed(&self, key: Key) -> bool {
        let pressed = self.pressed_no_intercept(key);
        if pressed {
            let _ = self.intercept_tx.try_send(KeyboardOp::InterceptKeypress);
        }
        pressed
    }

    pub fn pressed_no_intercept(&self, key: Key) -> bool {
        self.keys[0] == key
    }

    pub fn all_pressed(&self, keys: &[Key]) -> bool {
        let pressed = self.all_pressed_no_intercept(keys);
        if pressed {
            let _ = self.intercept_tx.try_send(KeyboardOp::InterceptKeypress);
        }
        pressed
    }

    pub fn all_pressed_no_intercept(&self, keys: &[Key]) -> bool {
        // Make sure len is the same otherwise we might match different keybind.
        keys.iter().all(|it| self.keys.iter().any(|k| *k == *it)) && self.keys.len() == keys.len()
    }
}
