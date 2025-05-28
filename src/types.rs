use std::collections::VecDeque;

use windows::Win32::{Foundation::*, Graphics::Gdi::HMONITOR, UI::WindowsAndMessaging::*};

use crate::input::{KBDelta, KeyState};

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

    pub(crate) focused_monitor: Monitor,
    pub(crate) focused_window: Option<Window>,
}

#[derive(Default, Debug)]
pub struct KeyMap {
    pub(crate) keys: [u32; 8],
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

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Self {
            x: r.left,
            y: r.top,
            width: r.right - r.left + 1,  // Make `Rect` inclusive.
            height: r.bottom - r.top + 1, // Make `Rect` inclusive.
        }
    }
}

impl From<CREATESTRUCTA> for Rect {
    fn from(value: CREATESTRUCTA) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.cx,
            height: value.cy,
        }
    }
}

impl From<Rect> for RECT {
    fn from(val: Rect) -> Self {
        RECT {
            top: val.y,
            left: val.x,
            bottom: val.y + val.height - 1, // Make `RECT` exclusive.
            right: val.x + val.width - 1,   // Make `RECT` exclusive.
        }
    }
}

impl Rect {
    #[inline]
    pub fn area(&self) -> i32 {
        self.width * self.height
    }

    pub fn intersection(&self, other: &Self) -> Self {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.width).min(other.x + other.width);
        let y2 = (self.y + self.height).min(other.y + other.height);

        if x1 < x2 && y1 < y2 {
            Self {
                x: x1,
                y: y1,
                width: x2 - x1,
                height: y2 - y1,
            }
        } else {
            Rect::default()
        }
    }

    pub fn center(&self) -> Point {
        let x = self.x + self.width / 2;
        let y = self.y + self.height / 2;
        Point { x, y }
    }

    pub fn scale(&self, scale: f64) -> Rect {
        Rect {
            x: (self.x as f64 * scale).round() as i32,
            y: (self.y as f64 * scale).round() as i32,
            width: (self.width as f64 * scale).round() as i32,
            height: (self.height as f64 * scale).round() as i32,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub fn distance(&self, other: Self) -> i32 {
        ((self.x - other.x).pow(2) as f32 + (self.y - other.y).pow(2) as f32).sqrt() as i32
    }
}

pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct WindowHandle {
    handle: usize,
}

impl From<HWND> for WindowHandle {
    fn from(handle: HWND) -> Self {
        Self {
            handle: handle.0 as _,
        }
    }
}

impl Into<HWND> for WindowHandle {
    fn into(self) -> HWND {
        HWND(self.handle as _)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct Window {
    pub(crate) index: usize,
    pub(crate) generation: usize,
}

impl Window {
    pub(crate) fn new(index: usize, generation: usize) -> Self {
        Self { index, generation }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct MonitorHandle {
    handle: usize,
}

impl From<HMONITOR> for MonitorHandle {
    fn from(handle: HMONITOR) -> Self {
        Self {
            handle: handle.0 as _,
        }
    }
}

impl Into<HMONITOR> for MonitorHandle {
    fn into(self) -> HMONITOR {
        HMONITOR(self.handle as _)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct Monitor {
    pub(crate) index: usize,
    pub(crate) generation: usize,
}

impl Monitor {
    pub(crate) fn new(index: usize, generation: usize) -> Self {
        Self { index, generation }
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub enum Layout {
    #[default]
    None,
    Stack,
    Grid,
    Full,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct WindowAttributes {
    // TODO: Use bits.
    pub minimized: bool,
    pub floating: bool,
}
