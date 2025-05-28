use std::collections::VecDeque;
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
use std::thread::JoinHandle;
use std::thread::{self};
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crossbeam::channel::{self, select, Receiver, Sender, TrySendError};

use crate::error::*;
use crate::input::*;
use crate::types::*;
use crate::wm::*;

// Messages for `.expect()` extracted here to declutter the actual code.
const COMPONENTS: &str = "if entity exists all of its components are present";
const RELATIONS: &str = "if window/monitor exists its related monitor/window must exist also";

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
    WindowFocusChange { window_handle: WindowHandle },
    // MonitorConnected(MonitorHandle),
    // MonitorDisconnected(MonitorHandle),
}

#[derive(Debug)]
pub enum Event {
    KeyPress(Input),
    WindowCreate(Window),
    WindowDestroy {
        window: Window,
        monitor: Monitor,
        title: String,
        rect: Rect,
        attributes: WindowAttributes,
    },
    WindowMoveOrResize {
        window: Window,
        old_rect: Rect,
        new_rect: Rect,
        old_monitor: Monitor,
        new_monitor: Monitor,
    },
    WindowFocusChange(Window),
}

pub struct EventQueue {
    msg_loop_thread_handle: JoinHandle<()>,

    win_event_rx: Receiver<WinEvent>,
    keyboard_delta_rx: Receiver<KBDelta>,
    keyboard_op_tx: Sender<KeyboardOp>,
}

impl EventQueue {
    pub fn new() -> (Self, Context) {
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
                (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
                (EVENT_OBJECT_FOCUS, EVENT_OBJECT_FOCUS),
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

        let s = Self {
            msg_loop_thread_handle,

            win_event_rx,
            keyboard_delta_rx,
            keyboard_op_tx,
        };

        let ctx = Context::new();
        return (s, ctx);
    }

    pub fn next_event(&mut self, ctx: &mut Context) -> Event {
        // We loop here in case we cannot return a meaningful event.
        loop {
            // `select!` macro messes up auto formatting.
            select! {
                recv(self.win_event_rx) -> event => {
                    if let Ok(event) = event {
                        if let Some(ev) = handle_windows_event(ctx, event) {
                            return ev;
                        }
                    }
                },
                recv(self.keyboard_delta_rx) -> kb_delta => {
                    match kb_delta {
                        Ok(kb_delta) => {
                            ctx.key_map.update(kb_delta);
                            let input = Input::new(ctx.key_map.keys, self.keyboard_op_tx.clone());
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
            PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0))
                .expect("thread id is correct and we did not hit message queue limit");
        }

        self.msg_loop_thread_handle
            .join()
            .expect("message thread did not panic");

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

fn handle_windows_event(ctx: &mut Context, event: WinEvent) -> Option<Event> {
    match event {
        WinEvent::WindowCreate { window_handle } => {
            if let Ok(w) = register_window(ctx, window_handle) {
                return Some(Event::WindowCreate(w));
            }
        }
        WinEvent::WindowDestroy { window_handle } => {
            if let Some(window) = window_from_handle(ctx, window_handle) {
                let monitor = monitor_from_window(ctx, window).expect(RELATIONS);
                let rect = window_rect(ctx, window).expect(COMPONENTS);
                let title = window_title(ctx, window).expect(COMPONENTS);
                let attributes = window_attributes(ctx, window).expect(COMPONENTS);

                unregister_window(ctx, window);

                return Some(Event::WindowDestroy {
                    window,
                    monitor,
                    rect,
                    title,
                    attributes,
                });
            }
        }
        WinEvent::WindowMoveOrResize { window_handle } => {
            if let Some(window) = window_from_handle(ctx, window_handle) {
                let new_rect = window_rect_live(window_handle).ok()?;
                let old_rect = window_rect(ctx, window).expect(COMPONENTS);

                let prev_monitor = monitor_from_window(ctx, window).expect(RELATIONS);
                let prev_monitor_handle = monitor_handle(ctx, prev_monitor).expect(COMPONENTS);
                let curr_monitor_handle = monitor_from_window_live(window_handle);
                let curr_monitor = monitor_from_handle(ctx, curr_monitor_handle).expect(COMPONENTS);

                update_window_rect(ctx, window, new_rect);

                // Update only when needed otherwise window queue order will change for no reason.
                if prev_monitor_handle != curr_monitor_handle {
                    update_window_monitor(ctx, window, curr_monitor);
                }

                return Some(Event::WindowMoveOrResize {
                    window,
                    old_rect,
                    new_rect,
                    old_monitor: prev_monitor,
                    new_monitor: curr_monitor,
                });
            }
        }
        WinEvent::WindowFocusChange { window_handle } => {
            if let Some(window) = window_from_handle(ctx, window_handle) {
                ctx.state.focused_window = Some(window);
                ctx.state.focused_monitor = monitor_from_window(ctx, window).expect(RELATIONS);
                return Some(Event::WindowFocusChange(window));
            }
        }
    }

    return None;
}

#[derive(Debug)]
pub struct Context {
    pub(crate) key_map: KeyMap,
    pub(crate) state: State,
}

impl Context {
    pub fn new() -> Self {
        let mut monitor_handles = Vec::new();
        monitors_live(&mut monitor_handles);
        let monitor_count = monitor_handles.len();

        let monitor_rects = monitor_handles
            .iter()
            .map(|h| monitor_rect_live(*h))
            .map(|r| r.unwrap_or_default())
            .collect();

        let mut monitor_windows = vec![VecDeque::<Window>::new(); monitor_count];

        let mut window_handles = Vec::new();
        windows_live(&mut window_handles);
        let window_count = window_handles.len();

        // NOTE: Compiler should merge this iterations (check this maybe?).
        let window_rects = window_handles
            .iter()
            .map(|h| window_rect_live(*h))
            .map(|r| r.unwrap_or_default())
            .collect();

        let window_monitor: Vec<Monitor> = window_handles
            .iter()
            .map(|h| monitor_from_window_live(*h))
            .map(|monitor| {
                monitor_handles
                    .iter()
                    .position(|h| *h == monitor)
                    .expect("these monitors are already collected by us")
            })
            .map(|idx| Monitor::new(idx, 0))
            .collect();

        for (i, monitor_idx) in window_monitor.iter().enumerate() {
            monitor_windows[monitor_idx.index].push_back(Window::new(i, 0));
        }

        let window_titles = window_handles
            .iter()
            .enumerate()
            .map(|(_, h)| window_title_live(*h).unwrap_or("".into()))
            .collect();

        let window_attributes = window_handles
            .iter()
            .enumerate()
            .map(|(_, h)| window_minimized_live(*h))
            .map(|min| WindowAttributes {
                minimized: min,
                floating: false,
            })
            .collect();

        let focused_window_handle = focused_window_live();
        let focused_window = window_handles
            .iter()
            .position(|h| Some(*h) == focused_window_handle)
            .map(|idx| Window::new(idx, 0));

        let focused_monitor = focused_window
            .map(|window| window_monitor[window.index])
            .unwrap_or(Monitor::new(0, 0));

        let state = State {
            monitor_handles,
            monitor_layouts: vec![Layout::None; monitor_count],
            monitor_windows,
            monitor_rects,
            monitor_generations: vec![0; monitor_count],
            monitor_slots: vec![Slot::Occupied; monitor_count],
            monitor_free_idx: None,

            window_handles,
            window_rects,
            window_titles,
            window_attributes,
            window_monitor,
            window_generations: vec![0; window_count],
            window_slots: vec![Slot::Occupied; window_count],
            window_free_idx: None,

            focused_window,
            focused_monitor,
        };

        Self {
            key_map: KeyMap::default(),
            state,
        }
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

    let is_window_event = id_object == OBJID_WINDOW.0 && id_child == 0 && !hwnd.is_invalid();
    if !is_window_event {
        return;
    }

    let is_destroy_event = event == EVENT_OBJECT_DESTROY;
    if !IsWindowVisible(hwnd).as_bool() && !is_destroy_event {
        return;
    }

    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

    if style & WS_CHILD.0 != 0 {
        return;
    }

    if ex_style & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0 {
        return;
    }

    let has_owner = GetWindow(hwnd, GW_OWNER).is_ok();
    let has_caption = style & WS_CAPTION.0 != 0;

    if has_owner && !has_caption {
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
        EVENT_SYSTEM_FOREGROUND | EVENT_OBJECT_FOCUS => WinEvent::WindowFocusChange {
            window_handle: WindowHandle::from(hwnd),
        },
        _ => return,
    };
    tracing::trace!("{:?}", win_event);

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
