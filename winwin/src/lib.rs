use allocator_api2::alloc::AllocError;
use allocator_api2::alloc::Allocator;
use allocator_api2::alloc::Global as GlobalAllocator;
use allocator_api2::vec::Vec;
use std::cell::Cell;
use std::collections::HashMap;
use std::collections::VecDeque;
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

pub struct Context<A: GenericAlloc = GlobalAllocator> {
    alloc: AllocContext<A>,
    cache: Cache,
}

impl Context<GlobalAllocator> {
    pub fn new() -> Self {
        let arena = Arena::new_with_global_alloc();
        let alloc = GlobalAllocator;
        let cache = Cache::default();

        Self {
            alloc: AllocContext {
                arena,
                general: alloc,
            },
            cache: cache,
        }
    }
}

pub trait GenericAlloc: Allocator + Copy {}
impl<T: Allocator + Copy> GenericAlloc for T {}

pub struct AllocContext<A: GenericAlloc = GlobalAllocator> {
    arena: Arena,
    general: A,
}

impl<A: GenericAlloc> Context<A> {
    pub fn init_cache(&mut self) {
        let monitors = monitors_live(self);
        let windows = windows_live(self);
        let mut window_queues = VecDeque::new();
        for monitor in monitors {
            let queue = windows
                .iter()
                .copied()
                .filter(|w| w.is_on_monitor(monitor))
                .collect();

            window_queues.push_back((monitor, queue));
        }
        drop(windows);

        self.cache.window_queues = window_queues;
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

    pub fn reset(&mut self) {
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
    key_map: KeyMap,
    monitor_layouts: HashMap<Monitor, Layout>,
    window_queues: VecDeque<(Monitor, VecDeque<Window>)>,
}

impl Cache {
    pub(crate) fn save_layout(&mut self, monitor: Monitor, layout: Layout) {
        self.monitor_layouts.insert(monitor, layout);
    }

    pub(crate) fn layout_on(&self, monitor: Monitor) -> Layout {
        *self.monitor_layouts.get(&monitor).unwrap_or(&Layout::None)
    }

    pub(crate) fn add_window_queue(&mut self, monitor: Monitor) {
        self.window_queues.push_back((monitor, VecDeque::new()));
    }

    pub(crate) fn add_window_to_queue(&mut self, window: Window, monitor: Monitor) {
        let queues = &mut self.window_queues;

        let target_queue_idx = queues
            .iter()
            .position(|(m, _)| *m == monitor)
            .expect("monitor must have its queue");
        let queue = &mut queues[target_queue_idx].1;
        queue.push_front(window);

        // We update monitor ordering.
        let queue = queues
            .remove(target_queue_idx)
            .expect("monitor must have its queue");
        queues.push_front(queue);
    }

    pub(crate) fn add_windows_to_queue(&mut self, windows: &[Window], monitor: Monitor) {
        let queue = &mut self
            .window_queues
            .iter_mut()
            .find(|(m, _)| *m == monitor)
            .expect("monitor must have its queue")
            .1;

        for window in windows {
            queue.push_front(*window);
        }
    }

    pub(crate) fn remove_window_from_queue(&mut self, window: Window, monitor: Monitor) {
        let queue = &mut self
            .window_queues
            .iter_mut()
            .find(|(m, _)| *m == monitor)
            .expect("monitor must have its queue")
            .1;
        queue.retain(|w| *w != window);
    }

    pub(crate) fn remove_windows_from_queue<'a, A>(&mut self, monitor: Monitor) {
        self.window_queues
            .iter_mut()
            .find(|(m, _)| *m == monitor)
            .expect("monitor must have its queue")
            .1
            .clear();
    }

    pub(crate) fn drain_windows_from_queue<'a, A>(
        &mut self,
        sink: &mut Vec<Window, A>,
        monitor: Monitor,
    ) where
        A: Allocator + Copy,
    {
        let queue = &mut self
            .window_queues
            .iter_mut()
            .find(|(m, _)| *m == monitor)
            .expect("monitor must have its queue")
            .1;
        sink.extend(queue.drain(..));
    }

    pub(crate) fn update_queue_order(&mut self, window: Window, monitor: Monitor) {
        let queue = &mut self
            .window_queues
            .iter_mut()
            .find(|(m, _)| *m == monitor)
            .expect("monitor must have its queue")
            .1;

        let target_window_idx = queue
            .iter()
            .position(|w| *w == window)
            .expect("window must be in this queue");

        queue
            .remove(target_window_idx)
            .expect("window must be in this queue");
        queue.push_front(window);
        // TODO: update monitor order
    }

    pub(crate) fn update_input(
        &mut self,
        kb_delta: KBDelta,
        command_tx: SyncSender<KeyboardOp>,
    ) -> Input {
        self.key_map.update(kb_delta);
        let input = self.key_map.input(command_tx);
        input
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

    pub fn input(&self, tx: SyncSender<KeyboardOp>) -> Input {
        let mut input = Input {
            keys: [Key::None; 10],
            intercept_tx: tx,
        };
        let mut slot = 0;

        for i in 0..256 {
            let idx = i / 32;
            let bit = i % 32;
            if self.keys[idx] & (1 << bit) != 0 {
                if slot > 10 {
                    input.keys[slot] = Key::from_vk_code(i as u8);
                    slot += 1;
                }
            }
        }

        input
    }
}

#[derive(Debug)]
pub struct Input {
    // Hold up to 10 pressed keys. No one makes shortcuts with more than 5.
    keys: [Key; 10],
    intercept_tx: SyncSender<KeyboardOp>,
}

impl Drop for Input {
    fn drop(&mut self) {
        let _ = self.intercept_tx.try_send(KeyboardOp::DoNothing);
    }
}

impl Input {
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
        let num_keys = self.keys.iter().filter(|k| **k != Key::None).count();
        keys.iter().all(|it| self.keys.iter().any(|k| *k == *it)) && num_keys == keys.len()
    }
}
