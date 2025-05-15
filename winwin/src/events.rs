use std::alloc::Allocator;
use std::os::windows::io::AsRawHandle;
use std::sync::{OnceLock, RwLock};
use std::thread::JoinHandle;
use std::thread::{self};
use windows::core::{s, w};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crossbeam::channel::{self, select, Receiver, Sender};

use crate::error::*;
use crate::input::*;
use crate::types::*;
use crate::wm;
use crate::Context;

static WIN_EVENT_TX: OnceLock<RwLock<Option<Sender<WinEvent>>>> = OnceLock::new();
static KEYBOARD_HANDLER: OnceLock<RwLock<Option<KeyboardHandler>>> = OnceLock::new();

struct KeyboardHandler {
    sender: Sender<KBDelta>,
    receiver: Receiver<KeyboardOp>,
}

pub enum KeyboardOp {
    InterceptKeypress,
    DoNothing,
}

enum WinEvent {
    WindowCreate { window: WindowHandle },
    WindowDestroy { window: WindowHandle },
    // WindowMonitorChanged(usize, usize),
    // WindowFocusHanged(usize),
    // MonitorConnected(usize),
    // MonitorDisconnected(usize),
}

#[derive(Debug)]
pub enum Event {
    KeyPress(Input),
    WindowCreate(Window),
    WindowDestroy(Window),
}

pub struct WindowManager {
    win_event_hooks: Vec<HWINEVENTHOOK>,
    keyboard_hook: HHOOK,
    msg_loop_thread_handle: JoinHandle<()>,

    win_event_rx: Receiver<WinEvent>,
    keyboard_delta_rx: Receiver<KBDelta>,
    keyboard_op_tx: Sender<KeyboardOp>,
}

impl WindowManager {
    // SAFETY: Caller must ensure that only one instance is created at a time.
    // It is safe to create another insance only after calling `shutdown` and waithing for it to finish.
    pub unsafe fn new() -> Self {
        let (win_event_tx, win_event_rx) = channel::bounded(100);
        WIN_EVENT_TX.set(RwLock::new(Some(win_event_tx)));

        let (keyboard_delta_tx, keyboard_delta_rx) = channel::bounded(10);
        let (keyboard_op_tx, keyboard_op_rx) = channel::bounded(0);

        let keyboard_handler = KeyboardHandler {
            sender: keyboard_delta_tx,
            receiver: keyboard_op_rx,
        };
        KEYBOARD_HANDLER.set(RwLock::new(Some(keyboard_handler)));

        let mut win_event_hooks = Vec::new();
        let events = [(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY)];
        for (ev_min, ev_max) in events {
            let hook = unsafe {
                SetWinEventHook(
                    ev_min,
                    ev_max,
                    None,
                    Some(win_event_hook_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };

            win_event_hooks.push(hook);
        }

        let keyboard_hook = unsafe {
            SetWindowsHookExA(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0).unwrap()
        };

        let msg_loop_thread_handle = thread::spawn(move || unsafe {
            let mut msg = MSG::default();
            if GetMessageW(&mut msg, None, 0, 0).as_bool() {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });

        Self {
            win_event_hooks,
            keyboard_hook,
            msg_loop_thread_handle,

            win_event_rx,
            keyboard_delta_rx,
            keyboard_op_tx,
        }
    }

    pub fn next_event<A: Allocator + Copy>(&mut self, ctx: &mut Context<A>) -> Event {
        // We loop here because we don't want to return to user code on events that we can handle by ourselves.
        loop {
            select! {
                recv(self.win_event_rx) -> event => {
                    match event {
                        Ok(WinEvent::WindowCreate { window }) => {
                            if let Ok(w) = ctx.register_window(window) {
                                return Event::WindowCreate(w);
                            }
                        }
                        Ok(WinEvent::WindowDestroy { window }) => {
                            let w = todo!();
                            return Event::WindowDestroy(w);
                        },
                        _ => {}
                   }
                },
                recv(self.keyboard_delta_rx) -> kb_delta => {
                    match kb_delta {
                        Ok(kb_delta) => {
                            ctx.update_input(kb_delta);
                            let input = ctx.input(self.keyboard_op_tx.clone());
                            return Event::KeyPress(input);
                        },
                        Err(_) => {}

                    }
                }
            };
        }
    }

    pub fn shutdown(self) {
        unsafe {
            let h = HANDLE(self.msg_loop_thread_handle.as_raw_handle() as _);
            let thread_id = GetThreadId(h);
            PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        tracing::trace!("shutdown done");
    }
}

unsafe extern "system" fn win_event_hook_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
    let is_window_event = id_object == OBJID_WINDOW.0 && id_child == 0 && hwnd != HWND::default();
    if !is_window_event {
        return;
    }

    let win_event = match event {
        EVENT_OBJECT_CREATE => WinEvent::WindowCreate {
            window: WindowHandle::from(hwnd),
        },
        EVENT_OBJECT_DESTROY => WinEvent::WindowDestroy {
            window: WindowHandle::from(hwnd),
        },
        _ => return,
    };

    let lock = WIN_EVENT_TX
        .get()
        .expect("OnceLock must be set up at this point");
    let guard = lock.read().expect("mutex is not poisoned");
    let tx = guard
        .as_ref()
        .expect("channel was set up and not cleaned up yet");

    tx.send(win_event);
}

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

        let lock = KEYBOARD_HANDLER
            .get()
            .expect("OnceLock must be set up at this point");
        let guard = lock.read().expect("mutex is not poisoned");
        let kb_handler = guard
            .as_ref()
            .expect("channel was set up and not cleaned up yet");

        let tx = &kb_handler.sender;
        let rx = &kb_handler.receiver;

        tx.send(kb_delta)
            .expect("main thread must still be around at this point");

        let op = rx
            .recv()
            .expect("hook thread must still be around at this point");

        if matches!(op, KeyboardOp::InterceptKeypress) {
            return LRESULT(-1);
        }
    }

    return CallNextHookEx(None, code, wparam, lparam);
}
