use std::alloc::Allocator;
use tracing::instrument;
use windows::core::BOOL;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::{Win32::Foundation::*, Win32::Graphics::Dwm::*, Win32::Graphics::Gdi::*};

use crate::error::Error;
use crate::map_err_b;
use crate::types::*;
use crate::Slot;
use crate::{map_err, Context};

pub fn monitor_from_window_live(window_handle: WindowHandle) -> MonitorHandle {
    let hmonitor = unsafe { MonitorFromWindow(window_handle.into(), MONITOR_DEFAULTTONEAREST) };
    MonitorHandle::from(hmonitor)
}

pub fn monitors_live<A>(monitors: &mut Vec<MonitorHandle, A>)
where
    A: Allocator + Copy,
{
    unsafe extern "system" fn enum_monitors_proc<A: Allocator + Copy>(
        hmonitor: HMONITOR,
        _lprc_clip: HDC,
        _lpfn_enum: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let dest_vec = lparam.0 as *mut Vec<MonitorHandle, A>;
        (*dest_vec).push(MonitorHandle::from(hmonitor));
        TRUE
    }

    let success = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitors_proc::<A>),
            LPARAM(monitors as *mut _ as isize),
        )
    };
    if !success.as_bool() {
        panic!("EnumDisplayMonitors should never fail");
    }
}

#[instrument(err, skip(ctx))]
pub fn monitor_from_window<A>(ctx: &Context<A>, window: Window) -> Result<Monitor, Error>
where
    A: Allocator + Copy,
{
    if !ctx.is_valid_window(window) {
        return Err(Error::BadWindow(window));
    }

    let monitor = ctx.state.window_monitor[window.index];
    return Ok(monitor);
}

pub fn monitors<A>(ctx: &mut Context<A>) -> Vec<Monitor>
where
    A: Allocator + Copy,
{
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
pub fn monitor_rect<A>(ctx: &mut Context<A>, monitor: Monitor) -> Result<Rect, Error>
where
    A: Allocator + Copy,
{
    if !ctx.is_valid_monitor(monitor) {
        return Err(Error::BadMonitor(monitor));
    }

    let rect = ctx.state.monitor_rects[monitor.index];
    return Ok(rect);
}

pub fn windows_live<A>(windows: &mut Vec<WindowHandle, A>)
where
    A: Allocator + Copy,
{
    unsafe extern "system" fn enum_windows_proc<A: Allocator + Copy>(
        hwnd: HWND,
        lparam: LPARAM,
    ) -> BOOL {
        if is_real_window(hwnd) {
            let dest_vec = lparam.0 as *mut Vec<WindowHandle, A>;
            (*dest_vec).push(WindowHandle::from(hwnd));
        }
        TRUE
    }

    unsafe {
        EnumWindows(
            Some(enum_windows_proc::<A>),
            LPARAM(windows as *mut _ as isize),
        )
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

pub fn windows<A>(ctx: &mut Context<A>) -> Vec<Window>
where
    A: Allocator + Copy,
{
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

pub fn windows_on<A>(ctx: &mut Context<A>, monitor: Monitor) -> Vec<Window>
where
    A: Allocator + Copy,
{
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
pub fn window_rect<A>(ctx: &mut Context<A>, window: Window) -> Result<Rect, Error>
where
    A: Allocator + Copy,
{
    if !ctx.is_valid_window(window) {
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

pub fn window_title<A>(ctx: &mut Context<A>, window: Window) -> Result<String, Error>
where
    A: Allocator + Copy,
{
    if !ctx.is_valid_window(window) {
        return Err(Error::BadWindow(window));
    }

    let title = ctx.state.window_titles[window.index].clone();
    return Ok(title);
}

pub fn window_minimized_live(window_handle: WindowHandle) -> bool {
    unsafe {
        return IsIconic(window_handle.into()).into();
    }
}

#[instrument(err, skip(ctx))]
pub fn layout_on<A>(ctx: &mut Context<A>, monitor: Monitor) -> Result<Layout, Error>
where
    A: Allocator + Copy,
{
    if !ctx.is_valid_monitor(monitor) {
        return Err(Error::BadMonitor(monitor));
    }

    let layout = ctx.state.monitor_layouts[monitor.index];
    return Ok(layout);
}

#[instrument(err, skip(ctx))]
pub fn apply_layout<A>(ctx: &mut Context<A>, monitor: Monitor, layout: Layout) -> Result<(), Error>
where
    A: Allocator + Copy,
{
    match layout {
        Layout::None => Ok(()),
        Layout::Stack => apply_stack_layout(ctx, monitor),
        Layout::Grid => apply_grid_layout(ctx, monitor),
        Layout::Full => apply_full_layout(ctx, monitor),
    }
}

#[instrument(err, skip(ctx))]
pub fn apply_stack_layout<A>(ctx: &mut Context<A>, monitor: Monitor) -> Result<(), Error>
where
    A: Allocator + Copy,
{
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
pub fn apply_grid_layout<A>(ctx: &mut Context<A>, monitor: Monitor) -> Result<(), Error>
where
    A: Allocator + Copy,
{
    todo!()
}

#[instrument(err, skip(ctx))]
pub fn apply_full_layout<A>(ctx: &mut Context<A>, monitor: Monitor) -> Result<(), Error>
where
    A: Allocator + Copy,
{
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
