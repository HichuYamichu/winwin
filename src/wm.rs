use allocator_api2::vec::*;
use windows::{Win32::Foundation::*, Win32::Graphics::Gdi::*, Win32::UI::WindowsAndMessaging::*};

use crate::{Arena, Context};

pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Window {
    pub handle: HWND,
}


impl Window {
    pub const NULL_HWND: HWND = HWND(0);
    pub const NULL: Window = Window { handle: Self::NULL_HWND };

    pub fn bounding_rect(&self) -> Rect {
        let mut rect = RECT::default();
        let res = unsafe { GetWindowRect(self.handle, &mut rect as *mut _) };
        match res {
            Ok(_) => rect.into(),
            Err(_) => Rect::default(),
        }
    }

    pub fn set_bounding_rect(&self, rect: Rect) {
        if rect == Rect::default() {
            return;
        }
        let res = unsafe {
            SetWindowPos(
                self.handle,
                Self::NULL_HWND,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE,
            )
        };
        assert!(res.is_ok());
    }

    pub fn is_on_monitor(&self, monitor: Monitor) -> bool {
        let wr = self.bounding_rect();
        let intersection = wr.intersection(&monitor.bounding_rect());

        let window_area = wr.area();
        let intersect_area = intersection.area();

        let overlap = intersect_area as f32 / window_area as f32;
        overlap >= 0.5
    }

    pub fn title(&self) -> String {
        unimplemented!()
        // let mut buff = [0; 255];
        // unsafe { GetWindowTextW(self.handle, &mut buff) };
        // String::from_utf16_lossy(&buff)
    }

    pub fn info(&self) -> WindowInfo {
        let mut info = WINDOWINFO {
            cbSize: core::mem::size_of::<WINDOWINFO>() as u32,
            ..Default::default()
        };
        let res = unsafe { GetWindowInfo(self.handle, &mut info) };
        // TODO: Handle can be null.
        assert!(res.is_ok());

        todo!()
    }

    pub fn is_null(&self) -> bool {
        *self == Self::NULL
    }
}

#[derive(Debug)]
pub struct WindowInfo {}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Default for Rect {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }
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

impl Into<RECT> for Rect {
    fn into(self) -> RECT {
        RECT {
            top: self.y,
            left: self.x,
            bottom: self.y + self.height,
            right: self.x + self.width,
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
}

pub fn get_focused_window() -> Window {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd == Window::NULL_HWND {
        return Window::NULL;
    }
    Window { handle: hwnd }
}

pub fn get_all_windows(ctx: &Context) -> Vec<Window, &Arena> {
    extern "system" fn push_visible_window(window: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let mut text: [u16; 512] = [0; 512];
            let len = GetWindowTextW(window, &mut text);
            let text = String::from_utf16_lossy(&text[..len as usize]);

            let mut info = WINDOWINFO {
                cbSize: core::mem::size_of::<WINDOWINFO>() as u32,
                ..Default::default()
            };
            // TODO: Remove this unwrap().
            GetWindowInfo(window, &mut info).unwrap();

            if !text.is_empty()
                && info.dwStyle.contains(WS_VISIBLE)
                && !info.dwStyle.contains(WS_POPUP)
            {
                let dest_vec = lparam.0 as *mut Vec<Window, &Arena>;
                (*dest_vec).push(Window { handle: window });
            }

            TRUE
        }
    }

    let mut windows = Vec::new_in(&ctx.arena);
    let res = unsafe {
        EnumWindows(
            Some(push_visible_window),
            LPARAM(&mut windows as *mut _ as isize),
        )
    };
    assert!(res.is_ok());

    windows
}

#[derive(Debug, Copy, Clone)]
pub struct Monitor {
    handle: HMONITOR,
}

impl Monitor {
    pub fn bounding_rect(&self) -> Rect {
        let mut info = MONITORINFO {
            cbSize: core::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };

        let success = unsafe { GetMonitorInfoW(self.handle, &mut info).as_bool() };
        assert!(success);
        info.rcMonitor.into()
    }
}

pub fn get_focused_monitor() -> Monitor {
    let window = get_focused_window();
    let handle = unsafe { MonitorFromWindow(window.handle, MONITOR_DEFAULTTOPRIMARY) };
    Monitor { handle }
}

pub fn get_monitor_with_window(window: Window) -> Monitor {
    let handle = unsafe { MonitorFromWindow(window.handle, MONITOR_DEFAULTTOPRIMARY) };
    Monitor { handle }
}

pub fn get_all_monitors(ctx: &Context) -> Vec<Monitor, &Arena> {
    unsafe extern "system" fn push_monitor(
        hmonitor: HMONITOR,
        _lprc_clip: HDC,
        _lpfn_enum: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let dest_vec = lparam.0 as *mut Vec<Monitor, &Arena>;
        (*dest_vec).push(Monitor { handle: hmonitor });

        TRUE
    }

    let mut monitors = Vec::new_in(&ctx.arena);
    let success: bool = unsafe {
        EnumDisplayMonitors(
            HDC(0),
            None,
            Some(push_monitor),
            LPARAM(&mut monitors as *mut _ as isize),
        )
        .into()
    };
    assert!(success);

    monitors
}

pub enum Layout {
    Stack,
    Grid,
    Full,
}

pub fn apply_layout(ctx: &Context, monitor: Monitor, layout: Layout) {
    match layout {
        Layout::Stack => set_stack_layout(ctx, monitor),
        Layout::Grid => set_stack_layout(ctx, monitor),
        Layout::Full => set_stack_layout(ctx, monitor),
    }
}

fn set_stack_layout(ctx: &Context, monitor: Monitor) {
    let mut windows = get_all_windows(ctx);
    windows.retain(|w| w.is_on_monitor(monitor));

    match windows.len() {
        0 => return,
        1 => set_full_layout(monitor),
        _ => {
            let monitor_rect = monitor.bounding_rect();
            let partitions_needed = windows.len() as i32 - 1;
            let partition_width = monitor_rect.width / 2;
            let partition_height = monitor_rect.height / partitions_needed;

            let main_window_rect = Rect {
                x: monitor_rect.x,
                y: monitor_rect.y,
                width: partition_width,
                height: monitor_rect.height,
            };

            let mut window_iter = windows.iter();
            let main_window = window_iter.next().expect("there are multiple windows");
            main_window.set_bounding_rect(main_window_rect);

            let mut sub_window_rect = Rect {
                x: main_window_rect.x + partition_width,
                y: 0,
                width: partition_width,
                height: partition_height,
            };

            for window in window_iter {
                window.set_bounding_rect(sub_window_rect);
                sub_window_rect.y += partition_height;
            }
        }
    }
}

fn set_full_layout(monitor: Monitor) {}

pub fn keep_layout(ctx: &Context, monitor: Monitor, window: Window) {}

pub fn move_focus(ctx: &Context, direction: Direction) {}
