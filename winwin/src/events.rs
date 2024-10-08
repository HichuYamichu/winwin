use allocator_api2::alloc::Allocator;
use core::slice;
use std::alloc;
use std::mem::MaybeUninit;
use std::sync::mpsc::{self, sync_channel};
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread::JoinHandle;
use std::thread::{self};
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Threading::*;
use windows::Win32::System::IO::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use winwin_common::{ClientEvent, SyncHandle};

use windows::core::{s, PCSTR};

use crate::{wm, Context, Input, KeyState, Monitor, Window};
pub use winwin_common::KBDelta;

const THREAD_POOL_SIZE: usize = 2;
const PIPE_INSTANCES_PER_WORKER: usize = 10;
const BUFFER_SIZE: usize = 512;
const PIPE_NAME: PCSTR = s!("\\\\.\\pipe\\winwin_pipe");

#[link(name = "hooks.dll", kind = "dylib")]
extern "system" {
    fn cbt_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    fn shell_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
}

pub enum KeyboardOp {
    InterceptKeypress,
    DoNothing,
}

pub enum Event<A: Allocator> {
    KeyPress(Input<A>),
    WindowOpen(Window, Monitor),
    WindowClose(Window, Monitor),
}

pub struct EventQueue {
    client_event_rx: Receiver<ClientEvent>,
    keyboard_tx: SyncSender<KeyboardOp>,

    // Used for shutdown and cleanup.
    iocp_handle: HANDLE,
    join_handles: [JoinHandle<()>; 2],
    hook_thread_id: u32,
}

impl EventQueue {
    // SAFETY: Caller must ensure that only one instance is created at a time.
    // It is safe to create another insance only after calling `shutdown` and waithing for it to finish.
    pub unsafe fn new(ctx: &Context) -> Self {
        // NOTE: Buffer should be big enough to handle spontaneous bursts of events.
        let (tx, rx) = mpsc::sync_channel(128);

        let iocp = create_io_completion_port();

        let (kb_tx, kb_rx) = mpsc::sync_channel(0);
        let (hook_thread_id_tx, hook_thread_id_rx) = sync_channel(0);
        let keyboard_hook_tx = tx.clone();

        let hook_thread_handle =
            thread::spawn(|| unsafe { install_hooks(keyboard_hook_tx, kb_rx, hook_thread_id_tx) });
        let pipe_server_handle = thread::spawn(move || unsafe { install_pipe_server(tx, iocp) });

        // This nonsense in necessary because Rust's ThreadId has nothing to do with actual thread id.
        let hook_thread_id = hook_thread_id_rx.recv().unwrap();

        ctx.cache.fill(ctx);

        Self {
            client_event_rx: rx,
            keyboard_tx: kb_tx,

            iocp_handle: *iocp,
            join_handles: [hook_thread_handle, pipe_server_handle],
            hook_thread_id,
        }
    }

    pub fn next_event<A>(&mut self, ctx: &Context<A>) -> Event<A>
    where
        A: Allocator + Copy,
    {
        // We loop here because we don't want to return to user code on events that we can handle by ourselves.
        loop {
            ctx.arena.reset();

            let event = self.client_event_rx.recv().unwrap();
            match event {
                ClientEvent::Keyboard(kb_delta) => {
                    let input = ctx
                        .cache
                        .update_input(ctx, kb_delta, self.keyboard_tx.clone());
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
                    let monitor = Monitor::from(monitor_handle);
                    ctx.cache.update_window_queue(monitor, window);
                }
                ClientEvent::WindowFocusHanged(window_handle) => {
                    let window = Window::from(window_handle);
                    let monitor = wm::get_monitor_with_window(ctx, window);
                    ctx.cache.update_window_queue(monitor, window);
                }
                ClientEvent::MonitorConnected(monitor_handle) => {
                    let monitor = Monitor::from(monitor_handle);
                    ctx.cache.add_window_queue(monitor);
                }
                ClientEvent::MonitorDisconnected(monitor_handle) => {
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
            // This will wake up all workder threads and cause them to shutdown.
            let _ = CloseHandle(self.iocp_handle);
        }

        for h in self.join_handles {
            let _ = h.join();
        }
        tracing::trace!("shutdown done");
    }
}

unsafe fn install_hooks(
    tx: SyncSender<ClientEvent>,
    rx: Receiver<KeyboardOp>,
    thread_id_tx: SyncSender<u32>,
) {
    thread_id_tx
        .send(GetCurrentThreadId())
        .expect("main thread is waiting for this id");
    KB_HANDLER.init(tx, rx);

    let main_h_instance =
        GetModuleHandleA(None).expect("loading handle to current exe should always succseed");
    let kb_hook = SetWindowsHookExA(
        WH_KEYBOARD_LL,
        Some(low_level_keyboard_proc),
        main_h_instance,
        0,
    )
    .unwrap();

    let dll_name = s!("hooks.dll");
    let h_instance =
        GetModuleHandleA(dll_name).expect("required dll has to be loaded at this point");

    let cbt_hook = SetWindowsHookExA(WH_CBT, Some(cbt_proc), h_instance, 0).unwrap();
    let shell_hook = SetWindowsHookExA(WH_SHELL, Some(shell_proc), h_instance, 0).unwrap();

    // GetMessageA will return once PostThreadMessageA in `EventQueue::shutdown` posts a message.
    let mut msg = MSG::default();
    let _ = GetMessageA(&mut msg as *mut _, None, 0, 0);

    let _ = UnhookWindowsHookEx(kb_hook);
    let _ = UnhookWindowsHookEx(cbt_hook);
    let _ = UnhookWindowsHookEx(shell_hook);

    tracing::trace!("hooks unloaded");
}

static mut KB_HANDLER: KeyboardHandler = KeyboardHandler::new();

struct KeyboardHandler {
    sender: MaybeUninit<SyncSender<ClientEvent>>,
    receiver: MaybeUninit<Receiver<KeyboardOp>>,
}

impl KeyboardHandler {
    const fn new() -> Self {
        Self {
            sender: MaybeUninit::uninit(),
            receiver: MaybeUninit::uninit(),
        }
    }
    fn init(&mut self, tx: SyncSender<ClientEvent>, rx: Receiver<KeyboardOp>) {
        self.sender.write(tx);
        self.receiver.write(rx);
    }

    unsafe fn get_sender(&self) -> &SyncSender<ClientEvent> {
        unsafe { self.sender.assume_init_ref() }
    }

    unsafe fn get_receiver(&self) -> &Receiver<KeyboardOp> {
        unsafe { self.receiver.assume_init_ref() }
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
        let tx = KB_HANDLER.get_sender();
        tx.send(event)
            .expect("main thread must still be around at this point");

        let rx = KB_HANDLER.get_receiver();
        let op = rx
            .recv()
            .expect("hook thread must still be around at this point");

        if matches!(op, KeyboardOp::InterceptKeypress) {
            return LRESULT(-1);
        }
    }

    return CallNextHookEx(None, code, wparam, lparam);
}

unsafe fn install_pipe_server(tx: SyncSender<ClientEvent>, iocp: SyncHandle) {
    let mut pool = IocpWorkerPool::new(iocp, tx);
    pool.start_workers_and_accept_connections();
}

struct IocpWorkerPool {
    event_tx: SyncSender<ClientEvent>,
    io_objects_pool: Receiver<usize>,
    io_objects_release_channel: Option<SyncSender<usize>>,

    // Alias to allocation holding all `IoData` objects. It is accessed only in drop impl and thus
    // is safe because drop is guaranteed to be run exclusively by one thread.
    allocation: usize,
    pipe_instance_count: usize,
    iocp: SyncHandle,
}

impl IocpWorkerPool {
    fn new(iocp: SyncHandle, event_tx: SyncSender<ClientEvent>) -> Self {
        let pipe_instance_count = PIPE_INSTANCES_PER_WORKER * THREAD_POOL_SIZE;
        let (io_objects_tx, io_objects_rx) = sync_channel(pipe_instance_count);

        let (allocation, slice) = unsafe {
            let layout =
                alloc::Layout::array::<IoData>(pipe_instance_count).expect("arguments are correct");
            let mem = alloc::alloc_zeroed(layout);
            let s = slice::from_raw_parts_mut::<IoData>(mem as *mut _, pipe_instance_count);
            (mem as usize, s)
        };

        for slot in slice.iter_mut() {
            let pipe = unsafe {
                CreateNamedPipeA(
                    PIPE_NAME,
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                    PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    BUFFER_SIZE as u32,
                    BUFFER_SIZE as u32,
                    0,
                    None,
                )
            }
            .unwrap();

            slot.pipe = pipe;
            let io_data_ptr = slot as *mut _;

            unsafe {
                CreateIoCompletionPort(pipe, *iocp, io_data_ptr as usize, 0).unwrap();
            }

            io_objects_tx
                .send(io_data_ptr as _)
                .expect("queue is big enough to hold every io_data");
        }

        Self {
            event_tx,
            io_objects_pool: io_objects_rx,
            io_objects_release_channel: Some(io_objects_tx),

            allocation,
            iocp,
            pipe_instance_count,
        }
    }

    fn start_workers_and_accept_connections(&mut self) {
        let io_objects_release_channel = self
            .io_objects_release_channel
            .take()
            .expect("channel was initialized");
        thread::scope(|s| {
            for _ in 0..THREAD_POOL_SIZE {
                let event_tx = self.event_tx.clone();
                let release_channel = io_objects_release_channel.clone();
                let iocp = self.iocp;
                s.spawn(move || unsafe { handle_iocp(iocp, release_channel, event_tx) });
            }

            // We drop this sender so that only senders left are ones owned by woker threads, this
            // enables the following loop to exit once all workers quit.
            drop(io_objects_release_channel);

            while let Some(io_data) = self.io_objects_pool.iter().next() {
                unsafe {
                    let io_data = &mut *(io_data as *mut IoData);
                    IoData::reset(io_data);
                    let pipe = io_data.pipe;
                    let overlapped = &mut io_data.overlapped as *mut _;
                    if let Err(e) = ConnectNamedPipe(pipe, Some(overlapped)) {
                        if e != ERROR_IO_PENDING.into() {
                            tracing::warn!(?e);
                        }
                    }
                }
            }
        });
    }
}

impl Drop for IocpWorkerPool {
    fn drop(&mut self) {
        let slice = unsafe {
            slice::from_raw_parts_mut::<IoData>(self.allocation as *mut _, self.pipe_instance_count)
        };
        for io_data in slice {
            let _ = unsafe { CloseHandle(io_data.pipe) };
        }

        let layout = alloc::Layout::array::<IoData>(self.pipe_instance_count)
            .expect("arguments are correct");
        unsafe { alloc::dealloc(self.allocation as *mut _, layout) };
        tracing::trace!("pool dropped");
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
struct IoData {
    overlapped: OVERLAPPED,
    pipe: HANDLE,
    buffer: [u8; BUFFER_SIZE],
    state: State,
}

impl IoData {
    fn reset(io_data: *mut Self) {
        unsafe {
            let _ = DisconnectNamedPipe((*io_data).pipe);
            (*io_data).overlapped = OVERLAPPED::default();
            (*io_data).buffer.as_mut_slice().fill(0);
            (*io_data).state = State::WaitingForConnection;
        }
    }

    unsafe fn as_usize(&mut self) -> usize {
        self as *mut _ as _
    }
}

impl Default for IoData {
    fn default() -> Self {
        IoData {
            overlapped: OVERLAPPED::default(),
            pipe: INVALID_HANDLE_VALUE,
            buffer: [0; BUFFER_SIZE],
            state: State::WaitingForConnection,
        }
    }
}

#[derive(Debug, Copy, Clone)]
enum State {
    WaitingForConnection,
    ReadEnqueued,
    WaitingForDisconnect,
}

unsafe fn handle_iocp(
    iocp: SyncHandle,
    io_objects_release_channel: SyncSender<usize>,
    event_tx: SyncSender<ClientEvent>,
) {
    let mut completion_key = 0;
    let mut bytes_transferred = 0;
    let mut overlapped = std::ptr::null_mut();

    loop {
        let result = GetQueuedCompletionStatus(
            iocp.0,
            &mut bytes_transferred,
            &mut completion_key,
            &mut overlapped,
            INFINITE,
        );

        if let Err(e) = result {
            if e == ERROR_ABANDONED_WAIT_0.into() || e == ERROR_INVALID_HANDLE.into() {
                // Thread pool is shutting down we need to quit.
                tracing::trace!("thread pool worker quit");
                break;
            } else if e == ERROR_BROKEN_PIPE.into() && !overlapped.is_null() {
                // Client disconnected, release this `io_data` and continue.
                let io_data = overlapped as usize;
                io_objects_release_channel
                    .send(io_data)
                    .expect("pool never quits before workers");
            } else {
                // Unexpected error.
                tracing::warn!(?e);
            }

            continue;
        }

        let io_data = &mut *(overlapped as *mut IoData);

        match io_data.state {
            State::WaitingForConnection => {
                // Connected
                match enqueue_pipe_read(io_data) {
                    Ok(_) => io_data.state = State::ReadEnqueued,
                    Err(e) => {
                        tracing::debug!(?e);
                        io_objects_release_channel
                            .send(io_data.as_usize())
                            .expect("pool never quits before workers");
                    }
                }
            }
            State::ReadEnqueued => {
                let client_event: ClientEvent =
                    bincode::deserialize(&io_data.buffer[..bytes_transferred as usize]).unwrap();
                event_tx
                    .send(client_event)
                    .expect("other end should not quit before this thread");

                io_objects_release_channel
                    .send(io_data as *mut _ as _)
                    .expect("pool never quits before workers");
            }
            State::WaitingForDisconnect => {
                unreachable!("client disconnection will be catched before this match statement");
            }
        }
    }
}

unsafe fn enqueue_pipe_read(io_data: &mut IoData) -> windows::core::Result<()> {
    let res = ReadFile(
        io_data.pipe,
        Some(io_data.buffer.as_mut_slice()),
        None,
        Some(&mut io_data.overlapped as *mut _),
    );

    if let Err(e) = res {
        if e != ERROR_IO_PENDING.into() {
            return Err(e);
        }
    }

    Ok(())
}

unsafe fn enqueue_pipe_write(io_data: &mut IoData) -> windows::core::Result<()> {
    let res = WriteFile(
        io_data.pipe,
        Some(&mut io_data.buffer),
        None,
        Some(&mut io_data.overlapped as *mut _),
    );

    if let Err(e) = res {
        if e != ERROR_IO_PENDING.into() {
            return Err(e);
        }
    }

    Ok(())
}

fn create_io_completion_port() -> SyncHandle {
    let iocp = unsafe {
        CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, THREAD_POOL_SIZE as _).unwrap()
    };
    SyncHandle(iocp)
}
