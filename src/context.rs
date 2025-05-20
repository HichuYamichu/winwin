use std::alloc::Allocator;
use std::alloc::System;
use std::collections::HashMap;
use std::collections::VecDeque;

use crossbeam::channel::Sender;
use tracing::instrument;

use crate::error::*;
use crate::events::KeyboardOp;
use crate::input::*;
use crate::types::*;
use crate::wm;

#[derive(Debug)]
pub struct Context<A: Allocator + Copy = System> {
    pub(crate) arena: A, // Spoof allocator for now, will be replaced once alloc lib is done.
    pub(crate) key_map: KeyMap,
    pub(crate) state: State,
    pub(crate) errors: Vec<Error>,
}

impl Context<System> {
    pub fn new() -> Self {
        Self::new_in(System)
    }
}

impl<A: Allocator + Copy> Context<A> {
    // TODO: clean this up.
    pub fn new_in(alloc: A) -> Self {
        let arena = alloc;

        let mut monitor_handles = Vec::new();
        wm::monitors_live(&mut monitor_handles);
        let monitor_count = monitor_handles.len();

        let monitor_layouts = vec![Layout::None; monitor_count];
        let mut monitor_windows = vec![VecDeque::<Window>::new(); monitor_count];
        let mut monitor_rects = Vec::with_capacity(monitor_count);
        for h in monitor_handles.iter() {
            let rect =
                wm::monitor_rect_live(*h).expect("these handles are valid; we just obtained them");
            monitor_rects.push(rect);
        }
        let monitor_generations = vec![0; monitor_count];
        let monitor_slots = vec![Slot::Occupied; monitor_count];

        let mut window_handles = Vec::new();
        wm::windows_live(&mut window_handles);
        let window_count = window_handles.len();

        let mut window_rects = Vec::with_capacity(window_count);
        let mut window_monitor = Vec::with_capacity(window_count);
        let mut window_titles = Vec::with_capacity(window_count);
        let mut window_attributes = Vec::with_capacity(window_count);
        for (i, h) in window_handles.iter().enumerate() {
            let rect =
                wm::window_rect_live(*h).expect("these handles are valid; we just obtained them");
            window_rects.push(rect);

            let title = wm::window_title_live(*h).unwrap_or("".into());
            window_titles.push(title);

            let minimized = wm::window_minimized_live(*h);
            let attributes = WindowAttributes {
                minimized,
                floating: false,
            };
            window_attributes.push(attributes);

            let monitor = wm::monitor_from_window_live(*h);
            let monitor_idx = monitor_handles
                .iter()
                .position(|h| *h == monitor)
                .expect("these monitors are already collected by us");
            window_monitor.push(Monitor::new(monitor_idx, 0));
            monitor_windows[monitor_idx].push_back(Window::new(i, 0));
        }
        let window_generations = vec![0; window_count];
        let window_slots = vec![Slot::Occupied; window_count];

        let state = State {
            monitor_handles,
            monitor_layouts,
            monitor_windows,
            monitor_rects,
            monitor_generations,
            monitor_slots,
            monitor_free_idx: None,

            window_handles,
            window_rects,
            window_titles,
            window_attributes,
            window_monitor,
            window_generations,
            window_slots,
            window_free_idx: None,
        };

        Self {
            arena,
            key_map: KeyMap::default(),
            state,
            errors: Vec::new(),
        }
    }

    pub(crate) fn is_valid_window(&self, window: Window) -> bool {
        self.state
            .window_generations
            .get(window.index)
            .map_or(false, |&gen| gen == window.generation)
    }

    pub(crate) fn is_valid_monitor(&self, monitor: Monitor) -> bool {
        self.state
            .monitor_generations
            .get(monitor.index)
            .map_or(false, |&gen| gen == monitor.generation)
    }

    pub(crate) fn window_from_handle(&self, window_handle: WindowHandle) -> Option<Window> {
        self.state
            .window_handles
            .iter()
            .position(|h| *h == window_handle)
            .map(|idx| Window {
                index: idx,
                generation: self.state.window_generations[idx],
            })
    }

    pub(crate) fn monitor_from_handle(&self, monitor_handle: MonitorHandle) -> Option<Monitor> {
        self.state
            .monitor_handles
            .iter()
            .position(|h| *h == monitor_handle)
            .map(|idx| Monitor {
                index: idx,
                generation: self.state.monitor_generations[idx],
            })
    }

    #[instrument(err, skip(self))]
    pub(crate) fn register_window(&mut self, window_handle: WindowHandle) -> Result<Window, Error> {
        tracing::trace!("register window: {:?}", window_handle);
        let monitor_handle = wm::monitor_from_window_live(window_handle);
        let monitor = self
            .monitor_from_handle(monitor_handle)
            .expect("this monitor must be managed by us");

        let window_rect = wm::window_rect_live(window_handle)?;
        let window_title = wm::window_title_live(window_handle)?;
        let window_minimized = wm::window_minimized_live(window_handle);
        let window_attributes = WindowAttributes {
            minimized: window_minimized,
            floating: false,
        };

        let idx = match self.state.window_free_idx {
            Some(idx) => {
                match self.state.window_slots[idx] {
                    Slot::Vaccant { next } => {
                        self.state.window_free_idx = next;
                        self.state.window_slots[idx] = Slot::Occupied;
                    }
                    Slot::Occupied => unreachable!(),
                }

                self.state.window_handles[idx] = window_handle;
                self.state.window_rects[idx] = window_rect;
                self.state.window_titles[idx] = window_title;
                self.state.window_attributes[idx] = window_attributes;
                self.state.window_monitor[idx] = monitor;
                idx
            }
            None => {
                let idx = self.state.window_handles.len();
                self.state.window_slots.push(Slot::Occupied);

                self.state.window_handles.push(window_handle);
                self.state.window_rects.push(window_rect);
                self.state.window_titles.push(window_title);
                self.state.window_attributes.push(window_attributes);
                self.state.window_monitor.push(monitor);
                self.state.window_generations.push(0);
                idx
            }
        };

        let window = Window::new(idx, self.state.window_generations[idx]);

        let monitor_idx = self
            .state
            .monitor_handles
            .iter()
            .position(|h| *h == monitor_handle)
            .expect("we must know about this monitor");
        self.state.monitor_windows[monitor_idx].push_back(window);
        return Ok(window);
    }

    pub(crate) fn unregister_window(&mut self, window: Window) {
        if !self.is_valid_window(window) {
            return;
        }

        tracing::trace!("register window: {:?}", window);
        let monitor = self.state.window_monitor[window.index];
        self.state.monitor_windows[monitor.index].retain(|w| *w != window);

        self.state.window_generations[window.index] += 1;
        self.state.window_slots[window.index] = Slot::Vaccant {
            next: self.state.window_free_idx,
        };

        self.state.window_free_idx = Some(window.index);
    }

    pub(crate) fn register_monitor(
        &mut self,
        monitor_handle: MonitorHandle,
    ) -> Result<Monitor, Error> {
        let monitor_rect = wm::monitor_rect_live(monitor_handle)?;
        let idx = match self.state.monitor_free_idx {
            Some(idx) => {
                match self.state.monitor_slots[idx] {
                    Slot::Vaccant { next } => {
                        self.state.monitor_free_idx = next;
                        self.state.monitor_slots[idx] = Slot::Occupied;
                    }
                    Slot::Occupied => unreachable!(),
                }

                self.state.monitor_handles[idx] = monitor_handle;
                self.state.monitor_layouts[idx] = Layout::None;
                self.state.monitor_windows[idx] = VecDeque::new();
                self.state.monitor_rects[idx] = monitor_rect;
                idx
            }
            None => {
                let idx = self.state.monitor_handles.len();
                self.state.monitor_slots.push(Slot::Occupied);

                self.state.monitor_handles.push(monitor_handle);
                self.state.monitor_layouts.push(Layout::None);
                self.state.monitor_windows.push(VecDeque::new());
                self.state.monitor_rects.push(monitor_rect);
                self.state.window_generations.push(0);
                idx
            }
        };

        let monitor = Monitor::new(idx, self.state.monitor_generations[idx]);
        return Ok(monitor);
    }

    pub(crate) fn unregister_monitor(&mut self) {
        // windows will have to be moved.
    }

    pub(crate) fn update_window_rect(&mut self, window: Window, rect: Rect) {
        if !self.is_valid_window(window) {
            return;
        }

        self.state.window_rects[window.index] = rect;
    }

    pub(crate) fn update_input(&mut self, kb_delta: KBDelta) {
        self.key_map.update(kb_delta)
    }

    pub(crate) fn input(&mut self, tx: Sender<KeyboardOp>) -> Input {
        Input::new(self.key_map.keys, tx)
    }
}

#[derive(Copy, Clone, Debug)]
pub(crate) enum Slot {
    Vaccant { next: Option<usize> },
    Occupied,
}

#[derive(Debug, Default)]
pub struct State {
    pub(crate) monitor_handles: Vec<MonitorHandle>,
    pub(crate) monitor_layouts: Vec<Layout>,
    pub(crate) monitor_windows: Vec<VecDeque<Window>>,
    pub(crate) monitor_rects: Vec<Rect>,
    pub(crate) monitor_generations: Vec<usize>,
    pub(crate) monitor_slots: Vec<Slot>,
    pub(crate) monitor_free_idx: Option<usize>,

    pub(crate) window_handles: Vec<WindowHandle>,
    pub(crate) window_rects: Vec<Rect>,
    pub(crate) window_titles: Vec<String>,
    pub(crate) window_monitor: Vec<Monitor>,
    pub(crate) window_attributes: Vec<WindowAttributes>,
    pub(crate) window_generations: Vec<usize>,
    pub(crate) window_slots: Vec<Slot>,
    pub(crate) window_free_idx: Option<usize>,
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
}
