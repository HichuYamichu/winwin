use std::alloc::Allocator;
use std::alloc::System;
use std::collections::HashMap;
use std::collections::VecDeque;

use crossbeam::channel::Sender;

use crate::error::*;
use crate::events::KeyboardOp;
use crate::input::*;
use crate::types::*;
use crate::wm;

pub struct Context<A: Allocator + Copy = System> {
    arena: A, // Spoof allocator for now, will be replaced once alloc lib is done.
    key_map: KeyMap,
    state: State,
    errors: Vec<Error>,
}

impl Context<System> {
    pub fn new() -> Self {
        let state = State::default();
        let arena = System;

        Self {
            arena,
            key_map: KeyMap::default(),
            state,
            errors: Vec::new(),
        }
    }
}

impl<A: Allocator + Copy> Context<A> {
    pub fn new_in(alloc: A) -> Self {
        todo!()
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

    pub(crate) fn save_layout(
        &mut self,
        monitor: Monitor,
        layout: Layout,
    ) -> Result<(), InvalidEntity> {
        if self.state.monitor_generations[monitor.index] == monitor.generation {
            self.state.monitor_layouts[monitor.index] = layout;
            return Ok(());
        } else {
            return Err(InvalidEntity);
        }
    }

    pub(crate) fn layout_on(&self, monitor: Monitor) -> Result<Layout, InvalidEntity> {
        if self.state.monitor_generations[monitor.index] != monitor.generation {
            return Err(InvalidEntity);
        }
        Ok(self.state.monitor_layouts[monitor.index])
    }

    pub(crate) fn register_window(&mut self, window_handle: WindowHandle) -> Result<Window, Error> {
        let monitor_handle = wm::monitor_live(window_handle);
        let window_rect = wm::rect_w_live(window_handle)?;
        let window_title = wm::title_live(window_handle)?;
        let idx = self.state.window_free_list.pop_front();
        let (idx, gen) = match idx {
            Some(idx) => {
                self.state.window_handles[idx] = window_handle;
                self.state.window_rects[idx] = window_rect;
                self.state.window_titles[idx] = window_title;
                (idx, self.state.window_generations[idx])
            }
            None => {
                self.state.window_handles.push(window_handle);
                self.state.window_rects.push(window_rect);
                self.state.window_titles.push(window_title);
                self.state.window_generations.push(0);
                (self.state.window_handles.len() - 1, 0)
            }
        };

        let window = Window {
            index: idx,
            generation: gen,
        };

        let monitor_idx = self
            .state
            .monitor_handles
            .iter()
            .position(|h| *h == monitor_handle)
            .expect("we must know about this monitor");
        self.state.monitor_windows[monitor_idx].push_back(window);
        return Ok(window);
    }

    pub(crate) fn update_input(&mut self, kb_delta: KBDelta) {
        self.key_map.update(kb_delta)
    }

    pub(crate) fn input(&mut self, tx: Sender<KeyboardOp>) -> Input {
        Input::new(self.key_map.keys, tx)
    }
}

#[derive(Default)]
pub struct State {
    monitor_handles: Vec<MonitorHandle>,
    monitor_layouts: Vec<Layout>,
    monitor_windows: Vec<VecDeque<Window>>,
    monitor_rects: Vec<Rect>,
    monitor_generations: Vec<usize>,
    monitor_free_list: VecDeque<usize>,

    window_handles: Vec<WindowHandle>,
    window_rects: Vec<Rect>,
    window_titles: Vec<String>,
    window_generations: Vec<usize>,
    window_free_list: VecDeque<usize>,
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
