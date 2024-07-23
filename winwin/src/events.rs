use allocator_api2::vec::*;
use core::slice;
use std::alloc;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread::Thread;
use std::{marker::PhantomData, thread};
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Threading::*;
use windows::Win32::System::IO::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use winwin_common::{ClientEvent, ServerCommand};

use crossbeam::atomic::AtomicCell;
use crossbeam::queue::ArrayQueue;

use windows_core::{s, PCSTR};

use crate::{Arena, Context, Key, KeyState, Window};
pub use winwin_common::{KBDelta, WindowEventKind};

const THREAD_POOL_SIZE: usize = 2;
const BUFFER_SIZE: usize = 512;
const PIPE_NAME: PCSTR = s!("\\\\.\\pipe\\winwin_pipe");

#[link(name = "hooks.dll", kind = "dylib")]
extern "system" {
    fn cbt_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    fn low_level_keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
}

pub enum Event<'a> {
    KeyPress(Input<'a>),
    WindowOpen(Window),
    WindowClose(Window),
    Shutdown,
}

pub struct EventQueue {
    ev_rx: Receiver<(ClientEvent, SyncSender<ServerCommand>)>,
    key_map: KeyMap,
}

impl EventQueue {
    pub fn new() -> Self {
        // NOTE: Buffer should be big enough to handle spontaneous bursts of events.
        let (tx, rx) = mpsc::sync_channel(128);

        thread::spawn(|| unsafe { install_pipe_server(tx) });
        thread::spawn(|| unsafe { install_hooks() });
        // seve this handles and join them on shutdown.

        Self {
            ev_rx: rx,
            key_map: KeyMap::default(),
        }
    }

    pub fn next_event<'a>(&mut self, ctx: &'a Context) -> Event<'a> {
        ctx.arena.reset();

        let (event, command_tx) = self.ev_rx.recv().unwrap();
        match event {
            ClientEvent::Keyboard(kb_delta) => {
                self.key_map.update(kb_delta);
                let input = self.key_map.input(ctx, command_tx);
                Event::KeyPress(input)
            }
            ClientEvent::CBT(handle, kind) => {
                command_tx
                    .send(ServerCommand::None)
                    .expect("Rx must be around");
                let window = Window {
                    handle: HWND(handle as _),
                };
                match kind {
                    WindowEventKind::Created => Event::WindowOpen(window),
                    WindowEventKind::Destroyed => Event::WindowClose(window),
                }
            }
        }
    }
}

unsafe fn install_pipe_server(tx: SyncSender<(ClientEvent, SyncSender<ServerCommand>)>) {
    let iocp = unsafe {
        CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, THREAD_POOL_SIZE as _).unwrap()
    };

    let pool = IoDataPool::new(10 * THREAD_POOL_SIZE, iocp);

    thread::scope(|s| {
        for _ in 0..THREAD_POOL_SIZE {
            let tx = tx.clone();
            s.spawn(|| unsafe { handle_pipe_client(iocp, &pool, tx) });
        }
        accept_pipe_connections(&pool);
    });

    // TODO: add atomic bool `is_shutting_down`
    // We only get here if there was as shutdown signal.
    // pool.free_all(); // Close pipes and io objects.
    // close iocp
}

unsafe fn handle_pipe_client(
    iocp: HANDLE,
    pool: &IoDataPool,
    tx: SyncSender<(ClientEvent, SyncSender<ServerCommand>)>,
) -> windows::core::Result<()> {
    let mut completion_key = 0;
    let mut bytes_transferred = 0;
    let mut overlapped = std::ptr::null_mut();

    loop {
        let result = GetQueuedCompletionStatus(
            iocp,
            &mut bytes_transferred,
            &mut completion_key,
            &mut overlapped,
            INFINITE,
        );

        if result.is_err() {
            if !overlapped.is_null() {
                let io_data = &mut *(overlapped as *mut IoData);
                pool.release(io_data);
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
                        pool.release(io_data);
                    }
                }
            }
            State::ReadEnqueued => {
                let client_message: ClientEvent =
                    bincode::deserialize(&io_data.buffer[..bytes_transferred as usize]).unwrap();
                let (command_tx, command_rx) = mpsc::sync_channel(0);
                tx.send((client_message, command_tx))
                    .expect("other end should not quit befor this thread");

                let command = command_rx.recv().unwrap();
                bincode::serialize_into(&mut io_data.buffer[..], &command).unwrap();

                match enqueue_pipe_write(io_data) {
                    Ok(_) => io_data.state = State::WriteEnqueued,
                    Err(e) => {
                        tracing::debug!(?e);
                        pool.release(io_data);
                    }
                }
            }
            State::WriteEnqueued => {
                // Enqueue dummy read so that client disconnection triggers iocp.
                match enqueue_pipe_read(io_data) {
                    Ok(_) => io_data.state = State::WaitingForDisconnect,
                    Err(e) => {
                        tracing::debug!(?e);
                        pool.release(io_data);
                    }
                }
            }
            State::WaitingForDisconnect => {
                unreachable!("client disconnection will be catched before this match statement");
            }
        }
    }

    Ok(())
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
        Some(&io_data.buffer[..]),
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

fn accept_pipe_connections(pool: &IoDataPool) {
    loop {
        unsafe {
            let io_data = pool.get();
            let pipe = (*io_data).pipe;
            let overlapped = &mut (*io_data).overlapped as *mut _;
            let _ = ConnectNamedPipe(pipe, Some(overlapped));
        };
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
            let _ = DisconnectNamedPipe((*io_data).pipe).unwrap();
            (*io_data).overlapped = OVERLAPPED::default();
            (*io_data).buffer.as_mut_slice().fill(0);
            (*io_data).state = State::WaitingForConnection;
        }
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
    WriteEnqueued,
    WaitingForDisconnect,
}

struct IoDataPool {
    queue: ArrayQueue<usize>,
    sleeper: AtomicCell<Option<Thread>>,
}

impl IoDataPool {
    fn new(size: usize, iocp: HANDLE) -> Self {
        let queue = ArrayQueue::new(size);

        let layout = alloc::Layout::array::<IoData>(size).expect("arguments are correct");
        let mem = unsafe { alloc::alloc(layout) };
        let mem = unsafe { slice::from_raw_parts_mut::<IoData>(mem as *mut _, size) };

        for slot in 0..size {
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

            mem[slot].pipe = pipe;
            let io_data_ptr = &mut mem[slot] as *mut _;

            unsafe {
                CreateIoCompletionPort(pipe, iocp, io_data_ptr as usize, 0).unwrap();
            }

            queue
                .push(io_data_ptr as _)
                .expect("queue is big enough to hold every io_data");
        }

        Self {
            queue,
            sleeper: AtomicCell::new(None),
        }
    }

    // SAFETY: `get` can only by called by one thread.
    unsafe fn get(&self) -> *mut IoData {
        loop {
            match self.queue.pop() {
                Some(data) => return data as *mut _,
                None => {
                    let thread_id = thread::current();
                    self.sleeper.store(Some(thread_id));
                    thread::park();
                }
            }
        }
    }

    fn release(&self, io_data: *mut IoData) {
        IoData::reset(io_data);
        self.queue
            .push(io_data as _)
            .expect("number of io_data objects is fixed thus this always succeeds");
        if let Some(sleeper) = self.sleeper.swap(None) {
            sleeper.unpark();
        }
    }
}

unsafe fn install_hooks() {
    let dll_name = s!("hooks.dll");
    let h_instance =
        GetModuleHandleA(dll_name).expect("required dll has to be loaded at this point");

    let _ = SetWindowsHookExA(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), h_instance, 0);
    let _ = SetWindowsHookExA(WH_CBT, Some(cbt_proc), h_instance, 0).unwrap();

    let _ = GetMessageA(std::ptr::null_mut(), None, 0, 0);
}

unsafe fn uninstall_hooks() {
    // UnhookWindowsHookEx()
}

pub struct KeyMap {
    keys: [u32; 8],
}

impl Default for KeyMap {
    fn default() -> Self {
        KeyMap { keys: [0; 8] }
    }
}

impl KeyMap {
    fn update(&mut self, kb_delta: KBDelta) {
        let idx = (kb_delta.vk_code / 32) as usize;
        let bit = kb_delta.vk_code % 32;
        match kb_delta.key_state {
            KeyState::Up => {
                self.keys[idx] &= !(1 << bit);
            }
            KeyState::Down => {
                self.keys[idx] |= 1 << bit;
            }
        }
    }

    fn input<'a>(&self, ctx: &'a Context, tx: SyncSender<ServerCommand>) -> Input<'a> {
        let mut pressed_keys = Vec::new_in(&ctx.arena);

        for i in 0..256 {
            let idx = i / 32;
            let bit = i % 32;
            if self.keys[idx] & (1 << bit) != 0 {
                pressed_keys.push(Key::from_vk_code(i as u8));
            }
        }

        Input {
            keys: pressed_keys,
            intercept_tx: tx,
        }
    }
}

#[derive(Debug)]
pub struct Input<'a> {
    keys: Vec<Key, &'a Arena>,
    intercept_tx: SyncSender<ServerCommand>,
}

impl<'a> Drop for Input<'a> {
    fn drop(&mut self) {
        let _ = self.intercept_tx.try_send(ServerCommand::None);
    }
}

impl Input<'_> {
    pub fn pressed(&self, key: Key) -> bool {
        let pressed = self.pressed_no_intercept(key);
        if pressed {
            let _ = self.intercept_tx.try_send(ServerCommand::InterceptKeypress);
        }
        pressed
    }

    pub fn pressed_no_intercept(&self, key: Key) -> bool {
        self.keys[0] == key
    }

    pub fn all_pressed(&self, keys: &[Key]) -> bool {
        let pressed = self.all_pressed_no_intercept(keys);
        if pressed {
            let _ = self.intercept_tx.try_send(ServerCommand::InterceptKeypress);
        }
        pressed
    }

    pub fn all_pressed_no_intercept(&self, keys: &[Key]) -> bool {
        // Make sure len is the same otherwise we might match different keybind.
        keys.iter().all(|it| self.keys.iter().any(|k| *k == *it)) && self.keys.len() == keys.len()
    }
}
