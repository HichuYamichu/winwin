#![feature(allocator_api)]
#![feature(get_mut_unchecked)]

use std::alloc::Allocator;
use std::alloc::Global;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::pin::Pin;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::mpsc::SyncSender;

use hylib::alloc::ArenaRef;
use hylib::alloc::{Arena, ArenaPin, ArenaPtr, GrowStrategy};
use hylib::{KiB, MiB};

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

#[derive(Debug)]
pub enum Error {
    A,
    B,
}

pub struct Context<A: Allocator = Global> {
    arena: Pin<Rc<Arena>>,
    persistent_arena: Arena,
    user_allocator: A,
    cache: Cache,
    errors: Option<Vec<Error, ArenaPin<Rc<Arena>>>>,
}

impl<A: Allocator> Context<A> {
    pub fn new(alloc: A) -> Self {
        let arena = Rc::pin(Arena::new(KiB(16), GrowStrategy::Chain));

        let persistent_arena: Arena = Arena::new(
            MiB(64),
            GrowStrategy::ReserveCommit {
                commit_size: KiB(16),
            },
        );

        let mut cache = Cache::default();

        let monitors = monitors_ex(&*arena, &cache);
        let windows = windows_ex(&*arena, &cache);

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

        cache.window_queues = window_queues;

        let errors_alloc = ArenaPin(arena.clone());
        let errors = Some(Vec::new_in(errors_alloc));

        Self {
            arena,
            persistent_arena,
            user_allocator: alloc,
            cache,
            errors,
        }
    }

    fn reset(&mut self) {
        // Errors must be dropped because their underlying allocator is being reset.
        let _ = self.errors.take();
        let mut pinned = Pin::into_inner(self.arena.clone());

        // SAFETY: see: https://doc.rust-lang.org/std/rc/struct.Rc.html#method.get_mut_unchecked
        // At this point exactly 2 `Rc`s exist, the original `self.arena` is not dereferenced nor borrowed for the duration returened reference is used.
        unsafe {
            debug_assert!(Rc::strong_count(&pinned) == 2);
            Rc::get_mut_unchecked(&mut pinned).reset();
        }

        let errors_alloc = ArenaPin(self.arena.clone());
        let errors = Some(Vec::new_in(errors_alloc));

        self.errors = errors;
    }
}

#[derive(Default)]
pub struct Cache {
    key_map: KeyMap,
    monitor_layouts: HashMap<Monitor, Layout>,
    window_queues: VecDeque<(Monitor, VecDeque<Window>)>,

    // monitors: Vec<Monitor>,
    // layouts: Vec<Layout>,
    // windows: Vec<Window>,
    // window_data: Vec<WindowData>,
    // window_queues: Vec<VecDeque<Window>>,
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

#[derive(Default, Debug)]
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
                if slot < 10 {
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
