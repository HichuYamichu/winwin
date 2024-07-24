use allocator_api2::vec::*;
use windows::{
    Win32::Foundation::*, Win32::Graphics::Gdi::*, Win32::System::Threading::*,
    Win32::UI::WindowsAndMessaging::*,
};
use winwin_common::Rect;

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
    pub const NULL_HWND: HWND = HWND(std::ptr::null_mut());
    pub const NULL: Window = Window {
        handle: Self::NULL_HWND,
    };

    pub fn rect(&self) -> Rect {
        let mut rect = RECT::default();
        let res = unsafe { GetWindowRect(self.handle, &mut rect as *mut _) };
        match res {
            Ok(_) => rect.into(),
            Err(_) => Rect::default(),
        }
    }

    pub fn set_rect(&self, rect: Rect) {
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
        let wr = self.rect();
        let intersection = wr.intersection(&monitor.bounding_rect());

        let window_area = wr.area();
        let intersect_area = intersection.area();

        let overlap = intersect_area as f32 / window_area as f32;
        overlap >= 0.5
    }

    // Only for debug purposes.
    pub fn title(&self) -> String {
        let mut buff = [0; 255];
        unsafe { GetWindowTextW(self.handle, &mut buff) };
        String::from_utf16_lossy(&buff)
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

    pub fn focus(&self) {
        unsafe {
            let current_thread_id = GetCurrentThreadId();
            let foreground_thread_id = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let _ = AttachThreadInput(current_thread_id, foreground_thread_id, TRUE);
            let _ = BringWindowToTop(self.handle);
            let _ = ShowWindow(self.handle, SW_SHOW);
            let _ = SetForegroundWindow(self.handle);
            let _ = AttachThreadInput(current_thread_id, foreground_thread_id, FALSE);
        };
    }

    pub fn is_null(&self) -> bool {
        *self == Self::NULL
    }
}

#[derive(Debug)]
pub struct WindowInfo {}

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
            let len = GetWindowTextLengthW(window);

            let mut info = WINDOWINFO {
                cbSize: core::mem::size_of::<WINDOWINFO>() as u32,
                ..Default::default()
            };
            let _ = GetWindowInfo(window, &mut info);

            if len != 0 && info.dwStyle.contains(WS_VISIBLE) && !info.dwStyle.contains(WS_POPUP) {
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

pub fn is_minimised(window: Window) -> bool {
    unsafe { IsIconic(window.handle).into() }
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
            HDC(std::ptr::null_mut()),
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
            main_window.set_rect(main_window_rect);

            let mut sub_window_rect = Rect {
                x: main_window_rect.x + partition_width,
                y: 0,
                width: partition_width,
                height: partition_height,
            };

            for window in window_iter {
                window.set_rect(sub_window_rect);
                sub_window_rect.y += partition_height;
            }
        }
    }
}

fn set_full_layout(_monitor: Monitor) {}

// pub fn keep_layout(ctx: &Context, monitor: Monitor, window: Window) {}

pub fn move_focus(ctx: &Context, direction: Direction) {
    let origin_window = get_focused_window();
    let target_window = get_adjacent(ctx, origin_window, direction);

    if let Some(target_window) = target_window {
        target_window.focus();
    }
}

pub fn swap_adjacent(ctx: &Context, window: Window, direction: Direction) {
    let other = get_adjacent(ctx, window, direction);

    if let Some(other) = other {
        let base_rect = window.rect();
        let other_rect = other.rect();

        window.set_rect(other_rect);
        other.set_rect(base_rect);
    }
}

pub fn get_adjacent(ctx: &Context, window: Window, direction: Direction) -> Option<Window> {
    let windows = get_all_windows(ctx);
    let origin_rect = window.rect();
    let origin_center = origin_rect.center();
    dbg!(origin_center);

    let mut candidate_windows = Vec::new_in(&ctx.arena);

    let window_centers = windows.iter().map(|w| (w, w.rect().center()));
    let windows_in_direction = window_centers
        .filter(|(w, _)| !is_minimised(**w))
        .filter(|(_, p)| match direction {
            Direction::Up => p.y > origin_center.y,
            Direction::Down => p.y <= origin_center.y,
            Direction::Left => p.x < origin_center.x,
            Direction::Right => p.x >= origin_center.x,
        })
        .filter(|(w, _)| **w != window);

    candidate_windows.extend(windows_in_direction);
    candidate_windows.sort_by(|(_, a), (_, b)| {
        let da = a.distance(origin_center);
        let db = b.distance(origin_center);
        da.cmp(&db)
    });

    candidate_windows.first().map(|(w, _)| **w)
}
