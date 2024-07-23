use std::sync::Once;
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::HRESULT;
use windows_core::{s, PCSTR};

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::Subscriber;

use winwin_common::{init_logger, log, ClientEvent, ServerCommand, WindowEventKind, LOGGER};

const PIPE_NAME: PCSTR = s!("\\\\.\\pipe\\winwin_pipe");
const BUFFER_SIZE: usize = 512;

static TRACING: Once = Once::new();

// Panicking in context of another process is an absolute no-go. All calls have to handle errors.

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
pub unsafe extern "system" fn cbt_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    TRACING.call_once(init_tracing);

    if code == HCBT_CREATEWND as _ {
        let hwnd = HWND(wparam.0 as _);
        let event = ClientEvent::CBT(hwnd.0 as _, WindowEventKind::Created);
        if let Err(e) = send_event(event) {
            tracing::warn!(?e);
        }

        if is_real_window(hwnd) {}
    }

    if code == HCBT_DESTROYWND as _ {
        let hwnd = HWND(wparam.0 as _);
        let event = ClientEvent::CBT(hwnd.0 as _, WindowEventKind::Destroyed);
        if let Err(e) = send_event(event) {
            tracing::warn!(?e);
        }
    }

    return CallNextHookEx(None, code, wparam, lparam);
}

#[no_mangle]
unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code == HC_ACTION as _ {
        // let kb_info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        // let kb_delta = KBDelta {
        //     vk_code: kb_info.vkCode as _,
        //     key_state: KeyState::from(wparam),
        // };
    }

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
    unsafe {
        let len = GetWindowTextLengthW(handle);

        let mut info = WINDOWINFO {
            cbSize: core::mem::size_of::<WINDOWINFO>() as u32,
            ..Default::default()
        };
        let _ = GetWindowInfo(handle, &mut info);
        tracing::info!(len, ?info.dwStyle);

        len != 0 && info.dwStyle.contains(WS_VISIBLE) && !info.dwStyle.contains(WS_POPUP)
    }
}

fn init_tracing() {
    let file_appender = RollingFileAppender::new(Rotation::DAILY, "C:\\winwin", "log_file.log");
    let subscriber = Subscriber::builder().with_writer(file_appender).finish();

    // There's nothing we can do if this fails.
    let _ = tracing::subscriber::set_global_default(subscriber);
}
