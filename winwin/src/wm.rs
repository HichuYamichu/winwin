use allocator_api2::vec::*;
use std::hash::{Hash, Hasher};
use std::mem::MaybeUninit;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::{Win32::Foundation::*, Win32::Graphics::Gdi::*, Win32::System::Threading::*};
use winwin_common::Rect;

use crate::{error_if, error_if_err, Arena, Context, IteratorCollectWithAlloc};

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

    pub fn set_rect(&self, rect: Rect) {
        if rect == Rect::default() {
            return;
        }

        unsafe {
            let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
            placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
            let res = GetWindowPlacement(self.handle, &mut placement);
            error_if_err!(res);

            if placement.showCmd == SW_MAXIMIZE.0 as _ {
                let res = PostMessageA(
                    self.handle,
                    WM_SYSCOMMAND,
                    WPARAM(SC_RESTORE as _),
                    LPARAM(0),
                );
                error_if_err!(res);
            }

            let res = SetWindowPos(
                self.handle,
                HWND::default(),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE,
            );
            error_if_err!(res);
        };
    }

    pub fn is_on_monitor(&self, monitor: Monitor) -> bool {
        let target_handle = unsafe { MonitorFromWindow(self.handle, MONITOR_DEFAULTTONULL) };
        return target_handle == monitor.handle;
    }

    pub fn title(&self) -> String {
        let mut buff = [0; 512];
        let end = unsafe { GetWindowTextW(self.handle, &mut buff) };
        String::from_utf16_lossy(&buff[..end as usize])
    }

    pub fn info(&self) -> WindowInfo {
        if self.is_invalid() {
            todo!()
        }

        let mut info = WINDOWINFO {
            cbSize: core::mem::size_of::<WINDOWINFO>() as u32,
            ..Default::default()
        };
        let res = unsafe { GetWindowInfo(self.handle, &mut info) };
        error_if_err!(res);

        todo!()
    }

    pub fn style(&self) -> WINDOW_STYLE {
        let style = unsafe { GetWindowLongW(self.handle, GWL_STYLE) };
        WINDOW_STYLE(style as _)
    }

    pub fn style_ex(&self) -> WINDOW_EX_STYLE {
        let style = unsafe { GetWindowLongW(self.handle, GWL_EXSTYLE) };
        WINDOW_EX_STYLE(style as _)
    }

    pub fn focus(&self) {
        unsafe {
            let current_thread_id = GetCurrentThreadId();
            let foreground_thread_id = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let success = AttachThreadInput(current_thread_id, foreground_thread_id, TRUE);
            error_if!(!success);

            error_if_err!(BringWindowToTop(self.handle));

            let _was_visible = ShowWindow(self.handle, SW_SHOW);
            let _was_set = SetForegroundWindow(self.handle);

            let success = AttachThreadInput(current_thread_id, foreground_thread_id, FALSE);
            error_if!(!success);
        };
    }

    pub fn maximize(&self) {
        unsafe {
            let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
            placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
            let res = GetWindowPlacement(self.handle, &mut placement);
            error_if_err!(res);

            if placement.showCmd == SW_MAXIMIZE.0 as _ {
                let res = PostMessageA(
                    self.handle,
                    WM_SYSCOMMAND,
                    WPARAM(SC_RESTORE as _),
                    LPARAM(0),
                );
                error_if_err!(res);
            }

            let res = PostMessageA(
                self.handle,
                WM_SYSCOMMAND,
                WPARAM(SC_MAXIMIZE as _),
                LPARAM(0),
            );
            error_if_err!(res);
        };
    }

    pub fn minimize(&self) {
        let res = unsafe {
            PostMessageA(
                self.handle,
                WM_SYSCOMMAND,
                WPARAM(SC_MINIMIZE as _),
                LPARAM(0),
            )
        };
        error_if_err!(res);
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
            let res = GetWindowInfo(window, &mut info);
            error_if_err!(res);

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
    match res {
        Ok(_) => windows,
        Err(e) => {
            tracing::error!(error = ?e);
            Vec::new_in(&ctx.arena)
        }
    }
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

        let success = unsafe { GetMonitorInfoW(self.handle, &mut info) };
        error_if!(!success);
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
    let success = unsafe {
        EnumDisplayMonitors(
            HDC(std::ptr::null_mut()),
            None,
            Some(push_monitor),
            LPARAM(&mut monitors as *mut _ as isize),
        )
    };
    error_if!(!success);

    monitors
}

fn adjust_for_non_client_area(
    target_rect: Rect,
    window_rect: Rect,
    client_rect: Rect,
    scale: f64,
) -> Rect {
    let border_width = ((window_rect.width - client_rect.width) / 2) as i32;
    let title_height = (window_rect.height - client_rect.height - border_width) as i32;

    Rect {
        x: (target_rect.x as f64 / scale).round() as i32 - border_width,
        y: (target_rect.y as f64 / scale).round() as i32 - title_height,
        width: (target_rect.width as f64 / scale).round() as i32 + border_width * 2,
        height: (target_rect.height as f64 / scale).round() as i32 + title_height + border_width,
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
    ctx.memory.remember_layout(monitor, layout);
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

            let windows_rect: Vec<Rect, &Arena> =
                windows.iter().map(|w| w.rect()).collect_with(&ctx.arena);
            let windows_client_rect: Vec<Rect, &Arena> = windows
                .iter()
                .map(|w| w.client_rect())
                .collect_with(&ctx.arena);
            let transformed_rects = ctx.arena.slice_mut_uninit::<Rect>(windows.len());

            transform_rects_for_stack_uninit(
                bounding_rect,
                scale,
                &windows_rect,
                &windows_client_rect,
                transformed_rects,
            );
            // let t: &mut [Rect] = unsafe { std::mem::transmute(transformed_rects) };
            // dbg!(&t);

            for (window, rect) in windows.iter().zip(transformed_rects.iter()) {
                window.set_rect(unsafe { rect.assume_init() });
            }
        }
    }
}

pub fn transform_rects_for_stack_uninit(
    bounding_rect: Rect,
    scale: f64,
    windows_rect: &[Rect],
    windows_client_rect: &[Rect],
    transformed_rects: &mut [MaybeUninit<Rect>],
) {
    let partitions_needed = windows_rect.len() as i32 - 1;
    let partition_width = (bounding_rect.width as f64 / 2.0 / scale).round() as i32;
    let partition_height =
        (bounding_rect.height as f64 / partitions_needed as f64 / scale).round() as i32;

    let main_rect = adjust_for_non_client_area(
        Rect {
            x: bounding_rect.x,
            y: bounding_rect.y,
            width: partition_width,
            height: bounding_rect.height,
        },
        windows_rect[0],
        windows_client_rect[0],
        scale,
    );
    transformed_rects[0].write(main_rect.scale(scale));

    for i in 1..windows_rect.len() {
        let sub_window_idx = i - 1;
        let rect = adjust_for_non_client_area(
            Rect {
                x: bounding_rect.x + partition_width,
                y: bounding_rect.y + sub_window_idx as i32 * partition_height,
                width: partition_width,
                height: partition_height,
            },
            windows_rect[i],
            windows_client_rect[i],
            scale,
        );
        transformed_rects[i].write(rect.scale(scale));
    }
}

pub fn transform_rects_for_stack(
    bounding_rect: Rect,
    scale: f64,
    windows_rect: &[Rect],
    windows_client_rect: &[Rect],
    transformed_rects: &mut [Rect],
) {
    transform_rects_for_stack_uninit(
        bounding_rect,
        scale,
        windows_rect,
        windows_client_rect,
        unsafe { std::mem::transmute(transformed_rects) },
    )
}

fn set_grid_layout(ctx: &Context, monitor: Monitor) {
    let windows = get_windows_on_monitor(ctx, monitor);

    match windows.len() {
        0 => return,
        1 => set_full_layout(ctx, monitor),
        _ => {
            let (dpi_x, _) = get_dpi_for_monitor(monitor);
            let scale = dpi_x as f64 / 96.0;
            let bounding_rect = monitor.rect();

            let windows_rect: Vec<Rect, &Arena> =
                windows.iter().map(|w| w.rect()).collect_with(&ctx.arena);
            let windows_client_rect: Vec<Rect, &Arena> = windows
                .iter()
                .map(|w| w.client_rect())
                .collect_with(&ctx.arena);
            let transformed_rects = ctx.arena.slice_mut_uninit::<Rect>(windows.len());

            transform_rects_for_grid_uninit(
                bounding_rect,
                scale,
                &windows_rect,
                &windows_client_rect,
                transformed_rects,
            );

            for (window, rect) in windows.iter().zip(transformed_rects.iter()) {
                window.set_rect(unsafe { rect.assume_init() });
            }
        }
    }
}

fn transform_rects_for_grid_uninit(
    bounding_rect: Rect,
    scale: f64,
    windows_rect: &[Rect],
    windows_client_rect: &[Rect],
    transformed_rects: &mut [MaybeUninit<Rect>],
) {
    let window_count = windows_rect.len();
    let rows = (window_count as f32).sqrt().ceil() as u32;
    let cols = (window_count as u32 + rows - 1) / rows;

    let cell_width = bounding_rect.width / cols as i32;
    let cell_height = bounding_rect.height / rows as i32;

    let fixup = window_count % 2;
    for i in 0..window_count as usize - fixup {
        let row = i as u32 / cols;
        let col = i as u32 % cols;

        let rect = adjust_for_non_client_area(
            Rect {
                x: bounding_rect.x + (col * cell_width as u32) as i32,
                y: bounding_rect.y + (row * cell_height as u32) as i32,
                width: cell_width,
                height: cell_height,
            },
            windows_rect[i],
            windows_client_rect[i],
            scale,
        );
        transformed_rects[i].write(rect);
    }

    if fixup != 0 {
        let last_idx = window_count - 1;
        let row = last_idx as i32 / cols as i32;
        let rect = adjust_for_non_client_area(
            Rect {
                x: bounding_rect.x,
                y: bounding_rect.y + row * cell_height,
                width: bounding_rect.width,
                height: cell_height,
            },
            windows_rect[last_idx],
            windows_client_rect[last_idx],
            scale,
        );
        transformed_rects[last_idx].write(rect);
    }
}

pub fn transform_rects_for_grid(
    bounding_rect: Rect,
    scale: f64,
    windows_rect: &[Rect],
    windows_client_rect: &[Rect],
    transformed_rects: &mut [Rect],
) {
    transform_rects_for_grid_uninit(
        bounding_rect,
        scale,
        windows_rect,
        windows_client_rect,
        unsafe { std::mem::transmute(transformed_rects) },
    )
}

fn set_full_layout(ctx: &Context, monitor: Monitor) {
    let windows = get_windows_on_monitor(ctx, monitor);
    for window in windows {
        window.maximize();
    }
}

pub fn move_focus(ctx: &Context, direction: Direction) {
    let origin_window = get_focused_window();
    let target_window = get_adjacent(ctx, origin_window, direction);

    if let Some(target_window) = target_window {
        target_window.focus();
    }
}

pub fn swap(w1: Window, w2: Window) {
    let w1_rect = w1.rect();
    let w2_rect = w2.rect();

    w1.set_rect(w2_rect);
    w2.set_rect(w1_rect);
}

pub fn swap_adjacent(ctx: &Context, window: Window, direction: Direction) {
    let other = get_adjacent(ctx, window, direction);

    if let Some(other) = other {
        swap(window, other);
    }
}

pub fn send(ctx: &Context, window: Window, monitor: Monitor) {
    let layout = ctx.memory.layout_on(monitor);
    let mut windows = get_windows_on_monitor(ctx, monitor);
    windows.push(window);

    let windows_rect: Vec<Rect, &Arena> = windows.iter().map(|w| w.rect()).collect_with(&ctx.arena);
    let windows_client_rect: Vec<Rect, &Arena> = windows
        .iter()
        .map(|w| w.client_rect())
        .collect_with(&ctx.arena);
    let transformed_rects = ctx.arena.slice_mut_uninit::<Rect>(windows.len());
    let bounding_rect = monitor.rect();
    let (dpi_x, _) = get_dpi_for_monitor(monitor);
    let scale = dpi_x as f64 / 96.0;

    match layout {
        Layout::None => {}
        Layout::Stack => {
            transform_rects_for_stack_uninit(
                bounding_rect,
                scale,
                &windows_rect,
                &windows_client_rect,
                transformed_rects,
            );
        }
        Layout::Full => {}
        Layout::Grid => {}
    }

    for (window, rect) in windows.iter().zip(transformed_rects.iter()) {
        window.set_rect(unsafe { rect.assume_init() });
    }
}

pub fn send_in(window: Window, direction: Direction) {
    todo!()
}

pub fn swap_or_send(window: Window, direction: Direction) {
    todo!()
}

pub fn swap_monitors(m1: Monitor, m2: Monitor) {
    todo!()
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

pub fn get_adjacent_monitor(ctx: &Context, monitor: Monitor, direction: Direction) {
    todo!()
}

pub fn kill_window(window: Window) {
    if window.is_invalid() {
        return;
    }

    let res = unsafe { PostMessageA(window.handle, WM_CLOSE, WPARAM(0), LPARAM(0)) };
    error_if_err!(res);
}

pub fn kill_all_windows(ctx: &Context) {
    let windows = get_all_windows(ctx);

    for window in windows {
        let res = unsafe { PostMessageA(window.handle, WM_CLOSE, WPARAM(0), LPARAM(0)) };
        error_if_err!(res);
    }
}
