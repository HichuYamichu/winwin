use allocator_api2::vec::*;
use std::hash::{Hash, Hasher};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::{Win32::Foundation::*, Win32::Graphics::Gdi::*, Win32::System::Threading::*};
use winwin_common::Rect;

use crate::{Arena, Context, IteratorCollectWithAlloc};

pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct Window {
    pub handle: HWND,
}

impl Window {
    pub fn rect(&self) -> Rect {
        let mut rect = RECT::default();
        let res = unsafe { GetWindowRect(self.handle, &mut rect as *mut _) };
        match res {
            Ok(_) => rect.into(),
            Err(_) => Rect::default(),
        }
    }

    pub fn client_rect(&self) -> Rect {
        let mut rect = RECT::default();
        let res = unsafe { GetClientRect(self.handle, &mut rect as *mut _) };
        match res {
            Ok(_) => rect.into(),
            Err(_) => Rect::default(),
        }
    }

    pub fn toolbar_height(&self) -> i32 {
        32
    }

    pub fn set_rect(&self, rect: Rect) {
        if rect == Rect::default() {
            return;
        }

        unsafe {
            let _ = PostMessageA(
                self.handle,
                WM_SYSCOMMAND,
                WPARAM(SC_RESTORE as _),
                LPARAM(0),
            );

            let _ = SetWindowPos(
                self.handle,
                HWND::default(),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE,
            );
        };
    }

    pub fn is_on_monitor(&self, monitor: Monitor) -> bool {
        let wr = self.rect();
        let intersection = wr.intersection(&monitor.rect());

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

    pub fn style(&self) -> WINDOW_STYLE {
        let style = unsafe { GetWindowLongW(self.handle, GWL_STYLE) };
        WINDOW_STYLE(style as _)
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

    pub fn maximize(&self) {
        unsafe {
            let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
            placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
            let _ = GetWindowPlacement(self.handle, &mut placement);

            if placement.showCmd == SW_MAXIMIZE.0 as _ {
                let _ = PostMessageA(
                    self.handle,
                    WM_SYSCOMMAND,
                    WPARAM(SC_RESTORE as _),
                    LPARAM(0),
                );
            }

            let _ = PostMessageA(
                self.handle,
                WM_SYSCOMMAND,
                WPARAM(SC_MAXIMIZE as _),
                LPARAM(0),
            );
        };
    }

    pub fn minimize(&self) {
        let _ = unsafe {
            PostMessageA(
                self.handle,
                WM_SYSCOMMAND,
                WPARAM(SC_MINIMIZE as _),
                LPARAM(0),
            );
        };
    }

    pub fn is_invalid(&self) -> bool {
        self.handle.is_invalid()
    }
}

#[derive(Debug)]
pub struct WindowInfo {}

pub fn get_focused_window() -> Window {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return Window::default();
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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct Monitor {
    handle: HMONITOR,
}

impl Monitor {
    pub fn rect(&self) -> Rect {
        let mut info = MONITORINFO {
            cbSize: core::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };

        let success = unsafe { GetMonitorInfoW(self.handle, &mut info).as_bool() };
        assert!(success);
        info.rcWork.into()
    }
}

impl Hash for Monitor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.handle.0.hash(state);
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

pub fn get_windows_on_monitor(ctx: &Context, monitor: Monitor) -> Vec<Window, &Arena> {
    let mut windows = get_all_windows(ctx);
    windows.retain(|w| w.is_on_monitor(monitor));
    windows
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

fn adjust_for_non_client_area(window: &Window, rect: Rect, scale: f64) -> Rect {
    let window_rect = window.rect();
    let client_rect = window.client_rect();
    let border_width = ((window_rect.width - client_rect.width) / 2) as i32;
    let title_height = (window_rect.height - client_rect.height - border_width) as i32;

    Rect {
        x: (rect.x as f64 / scale).round() as i32 - border_width,
        y: (rect.y as f64 / scale).round() as i32 - title_height,
        width: (rect.width as f64 / scale).round() as i32 + border_width * 2,
        height: (rect.height as f64 / scale).round() as i32 + title_height + border_width,
    }
}

fn get_dpi_for_monitor(monitor: Monitor) -> (u32, u32) {
    let mut dpi_x = 0;
    let mut dpi_y = 0;
    let _ = unsafe { GetDpiForMonitor(monitor.handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    (dpi_x, dpi_y)
}

#[derive(Default, Clone, Copy)]
pub enum Layout {
    #[default]
    None,
    Stack,
    Grid,
    Full,
}

pub fn apply_layout(ctx: &Context, monitor: Monitor, layout: Layout) {
    match layout {
        Layout::None => {}
        Layout::Stack => set_stack_layout(ctx, monitor),
        Layout::Grid => set_grid_layout(ctx, monitor),
        Layout::Full => set_full_layout(ctx, monitor),
    }
}

fn set_stack_layout(ctx: &Context, monitor: Monitor) {
    let windows = get_windows_on_monitor(ctx, monitor);

    match windows.len() {
        0 => return,
        1 => set_full_layout(ctx, monitor),
        _ => {
            let bounding_rect = monitor.rect();
            let (dpi_x, _) = get_dpi_for_monitor(monitor);
            let scale = dpi_x as f64 / 96.0;

            let partitions_needed = windows.len() as i32 - 1;
            let partition_width = (bounding_rect.width as f64 / 2.0 / scale).round() as i32;
            let partition_height =
                (bounding_rect.height as f64 / partitions_needed as f64 / scale).round() as i32;

            let main_window = &windows[0];
            let main_rect = adjust_for_non_client_area(
                main_window,
                Rect {
                    x: bounding_rect.x,
                    y: bounding_rect.y,
                    width: partition_width,
                    height: bounding_rect.height,
                },
                scale,
            );
            main_window.set_rect(main_rect.scale(scale));

            for (i, window) in windows[1..].iter().enumerate() {
                let rect = adjust_for_non_client_area(
                    window,
                    Rect {
                        x: bounding_rect.x + partition_width,
                        y: bounding_rect.y + i as i32 * partition_height,
                        width: partition_width,
                        height: partition_height,
                    },
                    scale,
                );
                window.set_rect(rect.scale(scale));
            }
        }
    }
}

fn set_grid_layout(ctx: &Context, monitor: Monitor) {
    let windows = get_windows_on_monitor(ctx, monitor);

    match windows.len() {
        0 => return,
        1 => set_full_layout(ctx, monitor),
        _ => {
            let mut window_rects = Vec::with_capacity_in(windows.len(), &ctx.arena);
            for window in windows.iter() {
                window_rects.push(window.rect());
            }

            let monitor_rect = monitor.rect();

            translate_rects_for_grid(monitor_rect, &windows, &mut window_rects);

            for (window, rect) in windows.into_iter().zip(window_rects.into_iter()) {
                window.set_rect(rect);
            }
        }
    }
}

fn translate_rects_for_grid(bounding_rect: Rect, windows: &[Window], rects: &mut [Rect]) {
    let window_count = windows.len() as i32;
    let rows = (window_count as f32).sqrt().ceil() as i32;
    let cols = (window_count + rows - 1) / rows;

    let cell_width = bounding_rect.width / cols;
    let cell_height = bounding_rect.height / rows;

    for (i, (rect, window)) in rects.iter_mut().zip(windows.iter()).enumerate() {
        let row = i as i32 / cols;
        let col = i as i32 % cols;

        let is_last_odd = i == window_count as usize - 1 && window_count % 2 == 1;

        let title_bar_height = window.toolbar_height();
        let style = window.style();

        let r = if is_last_odd {
            Rect {
                x: bounding_rect.x,
                y: bounding_rect.y + row * cell_height + title_bar_height,
                width: bounding_rect.width,
                height: cell_height - title_bar_height,
            }
        } else {
            Rect {
                x: bounding_rect.x + col * cell_width,
                y: bounding_rect.y + row * cell_height + title_bar_height,
                width: cell_width,
                height: cell_height - title_bar_height,
            }
        };

        *rect = r.adjusted(style);
    }
}

fn set_full_layout(ctx: &Context, monitor: Monitor) {
    let monitor_rect = monitor.rect();
    let windows = get_windows_on_monitor(ctx, monitor);
    for window in windows {
        window.maximize();
        // window.set_rect(monitor_rect);
    }
}

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

    let mut candidate_windows = Vec::new_in(&ctx.arena);

    let window_centers = windows.iter().map(|w| (w, w.rect().center()));
    let windows_in_direction = window_centers
        .filter(|(w, _)| !is_minimised(**w))
        .filter(|(_, p)| match direction {
            Direction::Up => p.y <= origin_center.y,
            Direction::Down => p.y > origin_center.y,
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
