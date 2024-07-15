use std::io::Write;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::OnceLock;
use std::thread;

use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::s;

use winwin_common::*;

#[no_mangle]
pub extern "C" fn add(left: usize, right: usize) -> usize {
    left + right
}

static mut INTERNAL_EVENT_SENDER: Option<SyncSender<InternalEvent>> = None;

pub static LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn make_logger() -> Logger {
    Logger::new("D:\\Code\\rust\\winwin\\winwin.log")
}

#[no_mangle]
pub unsafe extern "system" fn cbt_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let logger = LOGGER.get_or_init(make_logger);
    log!(logger, "cbt");

    if code == HSHELL_WINDOWCREATED as _ {
        if let Some(sender) = INTERNAL_EVENT_SENDER.as_ref() {
            let handle = HWND(wparam.0 as _);
            sender
                .send(InternalEvent::Shell(WindowEvent::Created, handle))
                .unwrap();
        }
    }

    if code == HSHELL_WINDOWDESTROYED as _ {
        if let Some(sender) = INTERNAL_EVENT_SENDER.as_ref() {
            let handle = HWND(wparam.0 as _);
            sender
                .send(InternalEvent::Shell(WindowEvent::Destroyed, handle))
                .unwrap();
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
        let kb_info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let kb_delta = KBDelta {
            vk_code: kb_info.vkCode as _,
            key_state: KeyState::from(wparam),
        };

        if let Some(sender) = INTERNAL_EVENT_SENDER.as_ref() {
            let (intercept_tx, intercept_rx) = mpsc::sync_channel(0);
            sender
                .send(InternalEvent::Keyboard(kb_delta, intercept_tx))
                .unwrap();

            if let Ok(do_intercept) = intercept_rx.recv() {
                if do_intercept {
                    return LRESULT(1);
                }
            }
        }
    }

    return CallNextHookEx(None, code, wparam, lparam);
}

#[no_mangle]
pub extern "C" fn init() -> Receiver<InternalEvent> {
    let (tx, rx) = mpsc::sync_channel(0);
    unsafe {
        INTERNAL_EVENT_SENDER = Some(tx);
    }

    rx
}
