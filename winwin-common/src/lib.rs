use std::ops::Deref;

use serde::{Deserialize, Serialize};

use windows::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

mod keys;
pub use keys::*;

/// This HANDLE must be safe to use from multiple threads.
#[derive(Copy, Clone, Debug)]
pub struct SyncHandle(pub HANDLE);

unsafe impl Sync for SyncHandle {}
unsafe impl Send for SyncHandle {}

impl Deref for SyncHandle {
    type Target = HANDLE;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ServerCommand {
    InterceptKeypress,
    ChangeWindowRect(Rect),
    None,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ClientEvent {
    WindowOpen(usize),
    WindowClose(usize),
    Keyboard(KBDelta),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KBDelta {
    pub vk_code: u8,
    pub key_state: KeyState,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
            width: r.right - r.left,
            height: r.bottom - r.top,
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
            bottom: val.y + val.height,
            right: val.x + val.width,
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

    pub fn adjusted(&self, style: WINDOW_STYLE) -> Self {
        let mut r = RECT::from(*self);
        unsafe { AdjustWindowRect(&mut r as *mut _, style, false).unwrap() };
        r.into()
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
