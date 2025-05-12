use std::alloc::Allocator;
use std::sync::mpsc::{self, sync_channel};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::thread::{self};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use winwin_common::ClientEvent;

use windows::core::{s, w};

use win_channel::{Config, Receiver as IPCReceiver, Sender as IPCSender};

use crate::{wm, Context, Input, KeyState, Monitor, Window};
pub use winwin_common::KBDelta;

const IPC_CHANNEL_SIZE: usize = 128;

const IPC_CONFIG: Config = Config {
    shmem_name: w!("winwin_shmem"),
    send_event_name: w!("winwin_send_event"),
    recv_event_name: w!("winwin_recv_event"),
    disconnect_event_name: w!("winwin_disconnect_event"),
};

#[link(name = "hooks.dll", kind = "dylib")]
extern "system" {
    fn cbt_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    fn shell_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
}

pub enum KeyboardOp {
    InterceptKeypress,
    DoNothing,
}

#[derive(Debug)]
pub enum Event {
    KeyPress(Input),
    WindowOpen(Window, Monitor),
    WindowClose(Window, Monitor),
}

pub struct WindowManager {
    client_event_rx: IPCReceiver<ClientEvent, { IPC_CHANNEL_SIZE }>,
    keyboard_ack_tx: SyncSender<KeyboardOp>,

    // Used for shutdown and cleanup.
    hook_thread_handle: JoinHandle<()>,
    hook_thread_id: u32,
}

impl WindowManager {
    // SAFETY: Caller must ensure that only one instance is created at a time.
    // It is safe to create another insance only after calling `shutdown` and waithing for it to finish.
    pub unsafe fn new() -> Self {
        let client_event_rx =
            IPCReceiver::<ClientEvent, IPC_CHANNEL_SIZE>::new(IPC_CONFIG).unwrap();
        let keyboard_hook_tx = IPCSender::<ClientEvent, IPC_CHANNEL_SIZE>::new(IPC_CONFIG).unwrap();

        let (keyboard_ack_tx, keyboard_ack_rx) = mpsc::sync_channel(0);
        let (hook_thread_id_tx, hook_thread_id_rx) = sync_channel(0);

        let hook_thread_handle = thread::spawn(move || unsafe {
            hook_thread_id_tx
                .send(GetCurrentThreadId())
                .expect("main thread is waiting for this id");

            install_hooks(keyboard_hook_tx, keyboard_ack_rx);
        });

        // This nonsense in necessary because Rust's ThreadId has nothing to do with actual thread id.
        let hook_thread_id = hook_thread_id_rx.recv().unwrap();

        Self {
            client_event_rx,
            keyboard_ack_tx,

            hook_thread_handle,
            hook_thread_id,
        }
    }

    pub fn next_event<A: Allocator + Copy>(&mut self, ctx: &mut Context<A>) -> Event {
        // We loop here because we don't want to return to user code on events that we can handle by ourselves.
        loop {
            for e in ctx.errors.iter() {
                dbg!(e);
            }
            ctx.reset();

            // SAFETY: This is the only receiver so long the user upheld safety requirements of `new`.
            let event = unsafe { self.client_event_rx.recv().unwrap() };

            match event {
                ClientEvent::Keyboard(kb_delta) => {
                    let input = ctx
                        .cache
                        .update_input(kb_delta, self.keyboard_ack_tx.clone());
                    return Event::KeyPress(input);
                }
                ClientEvent::WindowOpen(window_handle, monitor_handle) => {
                    let window = Window::from(window_handle);
                    let monitor = Monitor::from(monitor_handle);
                    ctx.cache.add_window_to_queue(window, monitor);
                    return Event::WindowOpen(window, monitor);
                }
                ClientEvent::WindowClose(window_handle, monitor_handle) => {
                    let window = Window::from(window_handle);
                    let monitor = Monitor::from(monitor_handle);
                    ctx.cache.remove_window_from_queue(window, monitor);
                    return Event::WindowClose(window, monitor);
                }
                ClientEvent::WindowMonitorChanged(window_handle, monitor_handle) => {
                    let window = Window::from(window_handle);
                    let old_monitor = wm::monitor_with_window(ctx, window);
                    let new_monitor = Monitor::from(monitor_handle);
                    ctx.cache.remove_window_from_queue(window, old_monitor);
                    ctx.cache.add_window_to_queue(window, new_monitor);
                }
                ClientEvent::WindowFocusHanged(window_handle) => {
                    let window = Window::from(window_handle);
                    let monitor = wm::monitor_with_window(ctx, window);
                    ctx.cache.update_queue_order(window, monitor);
                }
                ClientEvent::MonitorConnected(monitor_handle) => {
                    let monitor = Monitor::from(monitor_handle);
                    ctx.cache.add_window_queue(monitor);
                }
                ClientEvent::MonitorDisconnected(_monitor_handle) => {
                    // TODO: Entire cache has to be recomputed.
                }
            }
        }
    }

    // `shutdown` must be called explicitly before application can exit.
    pub fn shutdown(self) {
        unsafe {
            // This will unblock `install_hooks` thread which cleans up after itself.
            let _ = PostThreadMessageA(self.hook_thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        let _ = self.hook_thread_handle.join();
        tracing::trace!("shutdown done");
    }
}

unsafe fn install_hooks(tx: IPCSender<ClientEvent, IPC_CHANNEL_SIZE>, rx: Receiver<KeyboardOp>) {
    let mut kb_handler_guard = KB_HANDLER.lock().unwrap();
    *kb_handler_guard = Some(KeyboardHandler::new(tx, rx));
    drop(kb_handler_guard);

    let main_h_instance: HINSTANCE = GetModuleHandleA(None)
        .expect("loading handle to current exe should always succseed")
        .into();
    let kb_hook = SetWindowsHookExA(
        WH_KEYBOARD_LL,
        Some(low_level_keyboard_proc),
        Some(main_h_instance),
        0,
    )
    .unwrap();

    let dll_name = s!("hooks.dll");
    let h_instance: HINSTANCE = GetModuleHandleA(dll_name)
        .expect("required dll has to be loaded at this point")
        .into();

    let cbt_hook = SetWindowsHookExA(WH_CBT, Some(cbt_proc), Some(h_instance), 0).unwrap();
    let shell_hook = SetWindowsHookExA(WH_SHELL, Some(shell_proc), Some(h_instance), 0).unwrap();

    // GetMessageA will return once PostThreadMessageA in `EventQueue::shutdown` posts a message.
    let mut msg = MSG::default();
    let _ = GetMessageA(&mut msg as *mut _, None, 0, 0);

    let _ = UnhookWindowsHookEx(kb_hook);
    let _ = UnhookWindowsHookEx(cbt_hook);
    let _ = UnhookWindowsHookEx(shell_hook);

    let mut kb_handler_guard = KB_HANDLER.lock().unwrap();
    let kb_handler = kb_handler_guard.take();
    drop(kb_handler);

    tracing::trace!("hooks unloaded");
}

static KB_HANDLER: Mutex<Option<KeyboardHandler>> = Mutex::new(None);

struct KeyboardHandler {
    sender: IPCSender<ClientEvent, IPC_CHANNEL_SIZE>,
    receiver: Receiver<KeyboardOp>,
}

impl KeyboardHandler {
    fn new(tx: IPCSender<ClientEvent, IPC_CHANNEL_SIZE>, rx: Receiver<KeyboardOp>) -> Self {
        Self {
            sender: tx,
            receiver: rx,
        }
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

        let event = ClientEvent::Keyboard(kb_delta);
        let kb_handler_guard = KB_HANDLER.lock().unwrap();
        if let Some(ref kb_handler) = kb_handler_guard.as_ref() {
            let tx = &kb_handler.sender;
            let rx = &kb_handler.receiver;

            tx.send(event)
                .expect("main thread must still be around at this point");

            let op = rx
                .recv()
                .expect("hook thread must still be around at this point");

            if matches!(op, KeyboardOp::InterceptKeypress) {
                return LRESULT(-1);
            }
        }
    }

    return CallNextHookEx(None, code, wparam, lparam);
}
