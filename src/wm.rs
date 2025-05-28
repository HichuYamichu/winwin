use crossbeam::channel::Sender;
use std::collections::VecDeque;
use tracing::instrument;
use windows::core::BOOL;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::{Win32::Foundation::*, Win32::Graphics::Dwm::*, Win32::Graphics::Gdi::*};

use crate::error::Error;
use crate::events::KeyboardOp;
use crate::input::Input;
use crate::input::KBDelta;
use crate::map_err;
use crate::map_err_b;
use crate::types::*;
use crate::Context;

pub(crate) fn is_valid_window(ctx: &Context, window: Window) -> bool {
    ctx.state
        .window_generations
        .get(window.index)
        .map_or(false, |&gen| gen == window.generation)
}

pub(crate) fn is_valid_monitor(ctx: &Context, monitor: Monitor) -> bool {
    ctx.state
        .monitor_generations
        .get(monitor.index)
        .map_or(false, |&gen| gen == monitor.generation)
}

pub(crate) fn window_from_handle(ctx: &Context, window_handle: WindowHandle) -> Option<Window> {
    ctx.state
        .window_handles
        .iter()
        .position(|h| *h == window_handle)
        .map(|idx| Window {
            index: idx,
            generation: ctx.state.window_generations[idx],
        })
}

pub fn window_handle(ctx: &Context, window: Window) -> Result<WindowHandle, Error> {
    if !is_valid_window(ctx, window) {
        return Err(Error::BadWindow(window));
    }

    let window = ctx.state.window_handles[window.index];
    return Ok(window);
}

pub(crate) fn monitor_from_handle(ctx: &Context, monitor_handle: MonitorHandle) -> Option<Monitor> {
    ctx.state
        .monitor_handles
        .iter()
        .position(|h| *h == monitor_handle)
        .map(|idx| Monitor {
            index: idx,
            generation: ctx.state.monitor_generations[idx],
        })
}

pub fn monitor_handle(ctx: &Context, monitor: Monitor) -> Result<MonitorHandle, Error> {
    if !is_valid_monitor(ctx, monitor) {
        return Err(Error::BadMonitor(monitor));
    }

    let monitor = ctx.state.monitor_handles[monitor.index];
    return Ok(monitor);
}

#[instrument(err, skip(ctx))]
pub(crate) fn register_window(
    ctx: &mut Context,
    window_handle: WindowHandle,
) -> Result<Window, Error> {
    tracing::trace!("register window: {:?}", window_handle);
    let monitor_handle = monitor_from_window_live(window_handle);
    let monitor =
        monitor_from_handle(ctx, monitor_handle).expect("this monitor must be managed by us");

    let window_rect = window_rect_live(window_handle)?;
    let window_title = window_title_live(window_handle)?;
    let window_minimized = window_minimized_live(window_handle);
    let window_attributes = WindowAttributes {
        minimized: window_minimized,
        floating: false,
    };

    let idx = match ctx.state.window_free_idx {
        Some(idx) => {
            match ctx.state.window_slots[idx] {
                Slot::Vaccant { next } => {
                    ctx.state.window_free_idx = next;
                    ctx.state.window_slots[idx] = Slot::Occupied;
                }
                Slot::Occupied => unreachable!(),
            }

            ctx.state.window_handles[idx] = window_handle;
            ctx.state.window_rects[idx] = window_rect;
            ctx.state.window_titles[idx] = window_title;
            ctx.state.window_attributes[idx] = window_attributes;
            ctx.state.window_monitor[idx] = monitor;
            idx
        }
        None => {
            let idx = ctx.state.window_handles.len();
            ctx.state.window_slots.push(Slot::Occupied);

            ctx.state.window_handles.push(window_handle);
            ctx.state.window_rects.push(window_rect);
            ctx.state.window_titles.push(window_title);
            ctx.state.window_attributes.push(window_attributes);
            ctx.state.window_monitor.push(monitor);
            ctx.state.window_generations.push(0);
            idx
        }
    };

    let window = Window::new(idx, ctx.state.window_generations[idx]);

    let monitor_idx = ctx
        .state
        .monitor_handles
        .iter()
        .position(|h| *h == monitor_handle)
        .expect("we must know about this monitor");
    ctx.state.monitor_windows[monitor_idx].push_back(window);

    if focused_window_live() == Some(window_handle) {
        ctx.state.focused_window = Some(window);
    }

    return Ok(window);
}

pub(crate) fn unregister_window(ctx: &mut Context, window: Window) {
    if !is_valid_window(ctx, window) {
        return;
    }

    tracing::trace!("register window: {:?}", window);
    let monitor = ctx.state.window_monitor[window.index];
    ctx.state.monitor_windows[monitor.index].retain(|w| *w != window);

    if ctx.state.focused_window == Some(window) {
        let handle = focused_window_live();
        let window = handle.map(|h| window_from_handle(ctx, h)).flatten();
        ctx.state.focused_window = window;
    }

    ctx.state.window_generations[window.index] += 1;
    ctx.state.window_slots[window.index] = Slot::Vaccant {
        next: ctx.state.window_free_idx,
    };

    ctx.state.window_free_idx = Some(window.index);
}

pub(crate) fn update_window_rect(ctx: &mut Context, window: Window, rect: Rect) {
    if !is_valid_window(ctx, window) {
        return;
    }

    ctx.state.window_rects[window.index] = rect;
}

pub(crate) fn update_window_title(ctx: &mut Context, window: Window, title: String) {
    if !is_valid_window(ctx, window) {
        return;
    }

    ctx.state.window_titles[window.index] = title;
}

pub(crate) fn update_window_monitor(ctx: &mut Context, window: Window, new_monitor: Monitor) {
    if !is_valid_window(ctx, window) {
        return;
    }

    if !is_valid_monitor(ctx, new_monitor) {
        return;
    }

    let old_monitor = ctx.state.window_monitor[window.index];
    ctx.state.monitor_windows[old_monitor.index].retain(|w| *w != window);

    ctx.state.window_monitor[window.index] = new_monitor;
    ctx.state.monitor_windows[new_monitor.index].push_back(window);
}

pub(crate) fn register_monitor(
    ctx: &mut Context,
    monitor_handle: MonitorHandle,
) -> Result<Monitor, Error> {
    let monitor_rect = monitor_rect_live(monitor_handle)?;
    let idx = match ctx.state.monitor_free_idx {
        Some(idx) => {
            match ctx.state.monitor_slots[idx] {
                Slot::Vaccant { next } => {
                    ctx.state.monitor_free_idx = next;
                    ctx.state.monitor_slots[idx] = Slot::Occupied;
                }
                Slot::Occupied => unreachable!(),
            }

            ctx.state.monitor_handles[idx] = monitor_handle;
            ctx.state.monitor_layouts[idx] = Layout::None;
            ctx.state.monitor_windows[idx] = VecDeque::new();
            ctx.state.monitor_rects[idx] = monitor_rect;
            idx
        }
        None => {
            let idx = ctx.state.monitor_handles.len();
            ctx.state.monitor_slots.push(Slot::Occupied);

            ctx.state.monitor_handles.push(monitor_handle);
            ctx.state.monitor_layouts.push(Layout::None);
            ctx.state.monitor_windows.push(VecDeque::new());
            ctx.state.monitor_rects.push(monitor_rect);
            ctx.state.window_generations.push(0);
            idx
        }
    };

    let monitor = Monitor::new(idx, ctx.state.monitor_generations[idx]);
    return Ok(monitor);
}

pub(crate) fn unregister_monitor(ctx: &mut Context) {
    // windows will have to be moved.
}

pub fn monitor_from_window_live(window_handle: WindowHandle) -> MonitorHandle {
    let hmonitor = unsafe { MonitorFromWindow(window_handle.into(), MONITOR_DEFAULTTONEAREST) };
    MonitorHandle::from(hmonitor)
}

pub fn monitors_live(monitors: &mut Vec<MonitorHandle>) {
    unsafe extern "system" fn enum_monitors_proc(
        hmonitor: HMONITOR,
        _lprc_clip: HDC,
        _lpfn_enum: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let dest_vec = lparam.0 as *mut Vec<MonitorHandle>;
        (*dest_vec).push(MonitorHandle::from(hmonitor));
        TRUE
    }

    let success = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitors_proc),
            LPARAM(monitors as *mut _ as isize),
        )
    };
    if !success.as_bool() {
        panic!("EnumDisplayMonitors should never fail");
    }
}

#[instrument(err, skip(ctx))]
pub fn monitor_from_window(ctx: &Context, window: Window) -> Result<Monitor, Error> {
    if !is_valid_window(ctx, window) {
        return Err(Error::BadWindow(window));
    }

    let monitor = ctx.state.window_monitor[window.index];
    return Ok(monitor);
}

pub fn monitors(ctx: &mut Context) -> Vec<Monitor> {
    let monitors = ctx
        .state
        .monitor_slots
        .iter()
        .enumerate()
        .zip(ctx.state.monitor_generations.iter())
        .filter(|((_, s), _)| matches!(s, Slot::Occupied))
        .map(|((i, _), g)| Monitor::new(i, *g))
        .collect();

    return monitors;
}

pub fn focused_monitor(ctx: &Context) -> Monitor {
    return ctx.state.focused_monitor;
}

#[instrument(err)]
pub fn monitor_rect_live(monitor: MonitorHandle) -> Result<Rect, Error> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..unsafe { std::mem::zeroed() }
    };

    unsafe {
        map_err_b! {
            GetMonitorInfoW(monitor.into(), &mut info as *mut _),
            ERROR_INVALID_HANDLE => Error::BadMonitorHandle(monitor),
        }
    }
    return Ok(info.rcWork.into());
}

#[instrument(err, skip(ctx))]
pub fn monitor_rect(ctx: &mut Context, monitor: Monitor) -> Result<Rect, Error> {
    if !is_valid_monitor(ctx, monitor) {
        return Err(Error::BadMonitor(monitor));
    }

    let rect = ctx.state.monitor_rects[monitor.index];
    return Ok(rect);
}

pub fn windows_live(windows: &mut Vec<WindowHandle>) {
    unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if is_real_window(hwnd) {
            let dest_vec = lparam.0 as *mut Vec<WindowHandle>;
            (*dest_vec).push(WindowHandle::from(hwnd));
        }
        TRUE
    }

    unsafe {
        EnumWindows(Some(enum_windows_proc), LPARAM(windows as *mut _ as isize))
            .expect("this EnumWindows should never fail");
    };
}

pub fn is_real_window(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }

        let mut is_cloaked = 0;
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut is_cloaked as *mut i32 as *mut _,
            std::mem::size_of::<*mut u32>() as u32,
        )
        .expect("this call must not fail when all params are valid");
        if is_cloaked != 0 {
            return false;
        }

        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        if style & (WS_CHILD.0) != 0 {
            return false;
        }

        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0 {
            return false;
        }

        if GetWindow(hwnd, GW_OWNER).is_ok() && style & WS_CAPTION.0 != 0 {
            return false;
        }

        true
    }
}

pub fn is_visible_window(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

pub fn windows(ctx: &mut Context) -> Vec<Window> {
    let windows = ctx
        .state
        .window_slots
        .iter()
        .enumerate()
        .zip(ctx.state.window_generations.iter())
        .filter(|((_, s), _)| matches!(s, Slot::Occupied))
        .map(|((i, _), g)| Window::new(i, *g))
        .collect();

    return windows;
}

pub fn windows_on(ctx: &mut Context, monitor: Monitor) -> Vec<Window> {
    // let windows = ctx
    //     .state
    //     .window_slots
    //     .iter()
    //     .enumerate()
    //     .zip(ctx.state.window_generations.iter())
    //     .filter(|((_, s), _)| matches!(s, Slot::Occupied))
    //     .map(|((i, _), g)| Window::new(i, *g))
    //     .collect();

    todo!()
}

pub fn focused_window_live() -> Option<WindowHandle> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return None;
    } else {
        return Some(WindowHandle::from(hwnd));
    }
}

pub fn focused_window(ctx: &Context) -> Option<Window> {
    return ctx.state.focused_window;
}

#[instrument(err)]
pub fn window_rect_live(window_handle: WindowHandle) -> Result<Rect, Error> {
    unsafe {
        let mut r: RECT = std::mem::zeroed();
        map_err! {
            DwmGetWindowAttribute(
                window_handle.into(),
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut r as *mut _ as *mut _,
                std::mem::size_of::<RECT>() as u32,
            ),
            ERROR_INVALID_WINDOW_HANDLE => Error::BadWindowHandle(window_handle),
        };

        return Ok(r.into());
    }
}

#[instrument(err, skip(ctx))]
pub fn window_rect(ctx: &mut Context, window: Window) -> Result<Rect, Error> {
    if !is_valid_window(ctx, window) {
        return Err(Error::BadWindow(window));
    }

    let rect = ctx.state.window_rects[window.index];
    return Ok(rect);
}

#[instrument(err)]
pub fn window_title_live(window_handle: WindowHandle) -> Result<String, Error> {
    unsafe {
        let mut text: [u16; 512] = [0; 512];
        let length = GetWindowTextW(window_handle.into(), &mut text);
        Ok(String::from_utf16_lossy(&text[..length as usize]))
    }
}

pub fn window_title(ctx: &mut Context, window: Window) -> Result<String, Error> {
    if !is_valid_window(ctx, window) {
        return Err(Error::BadWindow(window));
    }

    let title = ctx.state.window_titles[window.index].clone();
    return Ok(title);
}

pub fn window_attributes(ctx: &mut Context, window: Window) -> Result<WindowAttributes, Error> {
    if !is_valid_window(ctx, window) {
        return Err(Error::BadWindow(window));
    }

    let attributes = ctx.state.window_attributes[window.index].clone();
    return Ok(attributes);
}

pub fn window_minimized_live(window_handle: WindowHandle) -> bool {
    unsafe {
        return IsIconic(window_handle.into()).into();
    }
}

#[instrument(err, skip(ctx))]
pub fn layout_on(ctx: &mut Context, monitor: Monitor) -> Result<Layout, Error> {
    if !is_valid_monitor(ctx, monitor) {
        return Err(Error::BadMonitor(monitor));
    }

    let layout = ctx.state.monitor_layouts[monitor.index];
    return Ok(layout);
}

#[instrument(err, skip(ctx))]
pub fn apply_layout(ctx: &mut Context, monitor: Monitor, layout: Layout) -> Result<(), Error> {
    match layout {
        Layout::None => Ok(()),
        Layout::Stack => apply_stack_layout(ctx, monitor),
        Layout::Grid => apply_grid_layout(ctx, monitor),
        Layout::Full => apply_full_layout(ctx, monitor),
    }
}

#[instrument(err, skip(ctx))]
pub fn apply_stack_layout(ctx: &mut Context, monitor: Monitor) -> Result<(), Error> {
    if !is_valid_monitor(ctx, monitor) {
        return Err(Error::BadMonitor(monitor));
    }

    let monitor_rect = ctx.state.monitor_rects[monitor.index];
    let windows = &ctx.state.monitor_windows[monitor.index];
    // Only consider non-minimized windows.
    let windows: Vec<Window> = windows
        .iter()
        .map(|w| (w, ctx.state.window_attributes[w.index]))
        .filter(|(_, a)| !a.minimized)
        .map(|(w, _)| *w)
        .collect();

    if windows.len() == 0 {
        return Ok(());
    }

    if windows.len() == 1 {
        return apply_full_layout(ctx, monitor);
    }

    let sector_width = monitor_rect.width / 2;
    let main_window_rect = Rect {
        x: monitor_rect.x,
        y: monitor_rect.y,
        width: sector_width,
        height: monitor_rect.height,
    };
    let main_window = windows[0];
    let main_window_handle = ctx.state.window_handles[main_window.index];

    let sub_window_count = windows.len() - 1;
    let sub_window_height = monitor_rect.height / sub_window_count as i32;

    apply_window_rect(main_window_handle, main_window_rect)?;

    let mut sub_window_rect = Rect {
        x: monitor_rect.x + sector_width,
        y: monitor_rect.y,
        width: sector_width,
        height: sub_window_height,
    };
    for (i, sub_window) in windows.iter().skip(1).enumerate() {
        sub_window_rect.y = i as i32 * sub_window_height;
        let sub_window_handle = ctx.state.window_handles[sub_window.index];
        apply_window_rect(sub_window_handle, sub_window_rect)?;
    }

    // Write layout after it is set in case of faliure;
    ctx.state.monitor_layouts[monitor.index] = Layout::Stack;

    Ok(())
}

#[instrument(err, skip(ctx))]
pub fn apply_grid_layout(ctx: &mut Context, monitor: Monitor) -> Result<(), Error> {
    if !is_valid_monitor(ctx, monitor) {
        return Err(Error::BadMonitor(monitor));
    }
    todo!()
}

#[instrument(err, skip(ctx))]
pub fn apply_full_layout(ctx: &mut Context, monitor: Monitor) -> Result<(), Error> {
    if !is_valid_monitor(ctx, monitor) {
        return Err(Error::BadMonitor(monitor));
    }
    todo!()
}

#[instrument(err)]
fn apply_window_rect(window: WindowHandle, rect: Rect) -> Result<(), Error> {
    unsafe {
        map_err! {
            SetWindowPos(
                window.into(),
                None,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
            ),
            ERROR_INVALID_WINDOW_HANDLE => Error::BadWindowHandle(window),
        }
    }

    Ok(())
}
