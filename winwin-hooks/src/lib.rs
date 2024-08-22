use windows::core::{s, PCSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::Subscriber;

use winwin_common::{ClientEvent, KBDelta, KeyState, Rect, ServerCommand};

const PIPE_NAME: PCSTR = s!("\\\\.\\pipe\\winwin_pipe");
const BUFFER_SIZE: usize = 512;

// Panicking in context of another process is an absolute no-go. All calls have to handle errors.

#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn DllMain(dll_module: HINSTANCE, call_reason: u32, _: *mut ()) -> bool {
    // TODO: Threads are racing to this log file.
    match call_reason {
        DLL_PROCESS_ATTACH => {
            let file_appender =
                RollingFileAppender::new(Rotation::DAILY, "C:\\winwin", "log_file.log");
            let subscriber = Subscriber::builder()
                .with_ansi(false)
                .with_writer(file_appender)
                .finish();

            // There's nothing we can do if this fails.
            let _ = tracing::subscriber::set_global_default(subscriber);
        }
        _ => {}
    }

    true
}

#[derive(Debug)]
enum HookError {
    #[allow(dead_code)]
    Windows(windows::core::Error),
    Serde,
}

impl From<windows::core::Error> for HookError {
    fn from(value: windows::core::Error) -> Self {
        Self::Windows(value)
    }
}

impl From<bincode::Error> for HookError {
    fn from(_value: bincode::Error) -> Self {
        Self::Serde
    }
}

#[no_mangle]
pub unsafe extern "system" fn shell_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HSHELL_WINDOWCREATED as _ {
        let hwnd = HWND(wparam.0 as _);

        if is_real_window(hwnd) {
            let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let event = ClientEvent::WindowOpen(hwnd.0 as _, hmonitor.0 as _);
            if let Err(e) = send_event(event) {
                tracing::warn!(?e);
            }
        }
    }

    if code == HSHELL_WINDOWDESTROYED as _ {
        let hwnd = HWND(wparam.0 as _);
        let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);

        let event = ClientEvent::WindowClose(hwnd.0 as _, hmonitor.0 as _);
        if let Err(e) = send_event(event) {
            tracing::warn!(?e);
        }
    }

    if code == HSHELL_MONITORCHANGED as _ {
        let hwnd = HWND(wparam.0 as _);
        let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);

        if is_real_window(hwnd) {
            let event = ClientEvent::WindowMonitorChanged(hwnd.0 as _, hmonitor.0 as _);
            if let Err(e) = send_event(event) {
                tracing::warn!(?e);
            }
        }
    }

    return CallNextHookEx(None, code, wparam, lparam);
}

#[no_mangle]
pub unsafe extern "system" fn cbt_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    return CallNextHookEx(None, code, wparam, lparam);
}

unsafe fn send_event(event: ClientEvent) -> Result<ServerCommand, HookError> {
    WaitNamedPipeA(PIPE_NAME, INFINITE)?;
    let pipe = unsafe {
        CreateFileA(
            PIPE_NAME,
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )?
    };

    let mut buffer = [0u8; BUFFER_SIZE];
    bincode::serialize_into(buffer.as_mut_slice(), &event)?;
    let mut bytes_written = 0;

    unsafe {
        WriteFile(
            pipe,
            Some(buffer.as_slice()),
            Some(&mut bytes_written),
            None,
        )?;
    }

    let mut bytes_read = 0;
    unsafe {
        ReadFile(pipe, Some(&mut buffer), Some(&mut bytes_read), None)?;
    };

    let server_command: ServerCommand = bincode::deserialize(&buffer[..bytes_read as usize])?;

    unsafe { CloseHandle(pipe)? };

    Ok(server_command)
}

fn is_real_window(handle: HWND) -> bool {
    let mut r = RECT::default();
    let _ = unsafe { GetWindowRect(handle, &mut r as *mut _) };

    let this_rect = Rect::from(r);
    if this_rect.x == i32::MAX
        || this_rect.y == i32::MAX
        || this_rect.width == i32::MAX
        || this_rect.height == i32::MAX
        || this_rect.x == i32::MIN
        || this_rect.y == i32::MIN
        || this_rect.width == i32::MIN
        || this_rect.height == i32::MIN
        || this_rect.width == 0
        || this_rect.height == 0
    {
        return false;
    }

    return true;
}
