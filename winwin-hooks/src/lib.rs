use std::sync::Once;
use windows::core::{s, PCSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::Subscriber;

use winwin_common::{ClientEvent, Rect, ServerCommand};

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
pub unsafe extern "system" fn cbt_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HCBT_CREATEWND as _ {
        let hwnd = HWND(wparam.0 as _);
        let create_wnd = &*(lparam.0 as *const CBT_CREATEWNDA);
        let creation_params = &*create_wnd.lpcs;
        let rect = Rect::from(*creation_params);

        if is_real_window(hwnd, creation_params) {
            let event = ClientEvent::WindowOpen(hwnd.0 as _, rect);
            if let Err(e) = send_event(event) {
                tracing::warn!(?e);
            }
        }
    }

    if code == HCBT_DESTROYWND as _ {
        // let hwnd = HWND(wparam.0 as _);

        // if is_real_window(hwnd, params) {
        //     let event = ClientEvent::WindowClose(hwnd.0 as _);
        //     if let Err(e) = send_event(event) {
        //         tracing::warn!(?e);
        //     }
        // }
    }

    return CallNextHookEx(None, code, wparam, lparam);
}

// This hook is called in context of main `winwin` process.
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
    // let write_buffer = bincode::serialize(&event)?;
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

fn is_real_window(_handle: HWND, params: &CREATESTRUCTA) -> bool {
    // let style = params.style;
    // let ex_style = params.dwExStyle;

    let this_rect = Rect::from(*params);
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

    if !params.hwndParent.is_invalid() && params.hwndParent != HWND_MESSAGE {
        return false;
    }
    //
    // if (style & WS_CHILD.0 as i32 != 0) || (style & WS_VISIBLE.0 as i32 == 0) {
    //     return false;
    // }
    //
    // if (ex_style & WS_EX_TOOLWINDOW).0 != 0 {
    //     return false;
    // }
    //
    // if !params.lpszClass.is_null() {
    //     if params.lpszClass.0 as usize & 0xFFFF == params.lpszClass.0 as usize {
    //         if params.lpszClass.0 as usize == 32768 {
    //             return false;
    //         }
    //     } else {
    //         let class_name = params.lpszClass;
    //         if class_name == s!("ToolbarWindow32")
    //             || class_name == s!("ComboBox")
    //             || class_name == s!("msctls_progress32")
    //         {
    //             return false;
    //         }
    //     }
    // }
    //
    // if params.cx <= 0 || params.cy <= 0 {
    //     return false;
    // }

    true
}
