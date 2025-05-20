use std::alloc::Allocator;
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
use std::thread::JoinHandle;
use std::thread::{self};
use windows::core::{s, w};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crossbeam::channel::{self, select, Receiver, Sender, TrySendError};

use crate::error::*;
use crate::input::*;
use crate::types::*;
use crate::wm;
use crate::Context;

static WIN_EVENT_TX: OnceLock<RwLock<Option<Sender<WinEvent>>>> = OnceLock::new();
static KEYBOARD_HANDLER: OnceLock<RwLock<Option<KeyboardHandler>>> = OnceLock::new();
static IS_INSTALLED: AtomicBool = AtomicBool::new(false);

struct KeyboardHandler {
    sender: Sender<KBDelta>,
    receiver: Receiver<KeyboardOp>,
}

pub enum KeyboardOp {
    InterceptKeypress,
    DoNothing,
}

#[derive(Debug)]
enum WinEvent {
    WindowCreate { window_handle: WindowHandle },
    WindowDestroy { window_handle: WindowHandle },
    WindowMoveOrResize { window_handle: WindowHandle },
    // WindowMonitorChanged(usize, usize),
    // WindowFocusHanged(usize),
    // MonitorConnected(usize),
    // MonitorDisconnected(usize),
}

#[derive(Debug)]
pub enum Event {
    KeyPress(Input),
    WindowCreate(Window),
    WindowDestroy(Window, Monitor),
    WindowMove {
        window: Window,
        old_rect: Rect,
        new_rect: Rect,
    },
    WindowResize {
        window: Window,
        old_rect: Rect,
        new_rect: Rect,
    },
}

pub struct WindowManager {
    msg_loop_thread_handle: JoinHandle<()>,

    win_event_rx: Receiver<WinEvent>,
    keyboard_delta_rx: Receiver<KBDelta>,
    keyboard_op_tx: Sender<KeyboardOp>,
}

impl WindowManager {
    pub fn new() -> Self {
        if IS_INSTALLED.load(Ordering::Acquire) {
            panic!("Second installation of `WindowManager` detected, aborting");
        }

        // Don't use unbounded channels. 128 should handle bursts of events just fine.
        let (win_event_tx, win_event_rx) = channel::bounded(128);
        if let Err(rejected) = WIN_EVENT_TX.set(RwLock::new(Some(win_event_tx))) {
            // `WIN_EVENT_TX` was initialized by previous run. We need to update it.
            let tx = rejected.into_inner().expect("there was no poisoning");
            let cell = WIN_EVENT_TX.get().expect("we know it is initialized");
            let mut lock = cell.write().expect("there was no poisoning");
            *lock = tx;
        }

        // WH_KEYBOARD_LL is not re-entrant so we don't need to buffer.
        let (keyboard_delta_tx, keyboard_delta_rx) = channel::bounded(0);
        let (keyboard_op_tx, keyboard_op_rx) = channel::bounded(0);

        let keyboard_handler = KeyboardHandler {
            sender: keyboard_delta_tx,
            receiver: keyboard_op_rx,
        };
        if let Err(rejected) = KEYBOARD_HANDLER.set(RwLock::new(Some(keyboard_handler))) {
            // `KEYBOARD_HANDLER` was initialized by previous run. We need to update it.
            let handler = rejected.into_inner().expect("there was no poisoning");
            let cell = KEYBOARD_HANDLER.get().expect("we know it is initialized");
            let mut lock = cell.write().expect("there was no poisoning");
            *lock = handler;
        }

        let msg_loop_thread_handle = thread::spawn(move || unsafe {
            let kb_hook =
                SetWindowsHookExA(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0).unwrap();

            let events = [
                (EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW),
                (EVENT_OBJECT_DESTROY, EVENT_OBJECT_DESTROY),
                (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
                // (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
                (EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZEEND),
            ];
            let mut win_event_hooks = Vec::new();
            for (ev_min, ev_max) in events {
                let hook = SetWinEventHook(
                    ev_min,
                    ev_max,
                    None,
                    Some(win_event_hook_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                );
                win_event_hooks.push(hook);
            }

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            let _ = UnhookWindowsHookEx(kb_hook);
            for hook in win_event_hooks {
                UnhookWinEvent(hook).expect("https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-unhookwinevent#return-value");
            }
        });

        Self {
            msg_loop_thread_handle,

            win_event_rx,
            keyboard_delta_rx,
            keyboard_op_tx,
        }
    }

    pub fn next_event<A: Allocator + Copy>(&mut self, ctx: &mut Context<A>) -> Event {
        // We loop here in case se cannot return a meaningful event.
        loop {
            // `select!` macro messes up auto formatting.
            select! {
                recv(self.win_event_rx) -> event => {
                    match event {
                        Ok(WinEvent::WindowCreate { window_handle }) => {
                            if let Ok(w) = ctx.register_window(window_handle) {
                                return Event::WindowCreate(w);
                            }
                        }
                        Ok(WinEvent::WindowDestroy { window_handle }) => {
                            if let Some(window) = ctx.window_from_handle(window_handle) {
                                let monitor = wm::monitor_from_window(ctx, window)
                                    .expect("if we found this window it must belong to a monitor we manage");
                                ctx.unregister_window(window);
                                return Event::WindowDestroy(window, monitor);
                            }
                        },
                        Ok(WinEvent::WindowMoveOrResize { window_handle }) => {
                            if let Some(window) = ctx.window_from_handle(window_handle) {
                                let old_rect = wm::window_rect(ctx, window)
                                    .expect("if we found this window we must have all of its components");
                                let Ok(new_rect) = wm::window_rect_live(window_handle) else {
                                    continue;
                                };
                                ctx.update_window_rect(window, new_rect);

                                // let moved = old_rect.x != new_rect.x 
                                //  || old_rect.y != new_rect.y;
                                let resized = old_rect.width != new_rect.width
                                    || old_rect.height != new_rect.height;

                                if resized {
                                    return Event::WindowResize { window, old_rect, new_rect };
                                } else {
                                    return Event::WindowMove { window, old_rect, new_rect };
                                }
                            }
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

    pub fn shutdown(self) -> Result<(), Error> {
        unsafe {
            let h = HANDLE(self.msg_loop_thread_handle.as_raw_handle() as _);
            let thread_id = GetThreadId(h);
            PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }

        self.msg_loop_thread_handle.join();

        let lock = KEYBOARD_HANDLER
            .get()
            .expect("OnceLock must be set up at this point");
        let mut guard = lock.write().expect("mutex is not poisoned");
        *guard = None;

        let lock = WIN_EVENT_TX
            .get()
            .expect("OnceLock must be set up at this point");
        let mut guard = lock.write().expect("mutex is not poisoned");
        *guard = None;

        IS_INSTALLED.store(false, Ordering::Release);

        tracing::trace!("shutdown done");

        Ok(())
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
    use windows::Win32::UI::WindowsAndMessaging::*;
    let is_window_event = id_object == OBJID_WINDOW.0 && id_child == 0 as _ && !hwnd.is_invalid();

    if !is_window_event {
        return;
    }

    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    let is_popup = style & WS_POPUP.0 != 0;
    let has_owner = !GetWindow(hwnd, GW_OWNER).unwrap_or_default().is_invalid();
    let has_parent = !GetParent(hwnd).unwrap_or_default().is_invalid();

    // Seems like we need to check for parents even if id_child is 0.
    if is_popup || has_owner || has_parent {
        return;
    }

    let win_event = match event {
        EVENT_OBJECT_SHOW => WinEvent::WindowCreate {
            window_handle: WindowHandle::from(hwnd),
        },
        EVENT_OBJECT_DESTROY => WinEvent::WindowDestroy {
            window_handle: WindowHandle::from(hwnd),
        },
        EVENT_SYSTEM_MOVESIZEEND | EVENT_OBJECT_LOCATIONCHANGE => WinEvent::WindowMoveOrResize {
            window_handle: WindowHandle::from(hwnd),
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

    // We drop events if we cant handle them fast enough.
    if let Err(TrySendError::Full(ev)) = tx.try_send(win_event) {
        tracing::warn!("Event dropped: {:?}", ev);
    }
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
