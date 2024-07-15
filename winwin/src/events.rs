use allocator_api2::vec::*;
use std::ffi::CString;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, SyncSender};
use std::{marker::PhantomData, thread};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use windows_core::s;

use crate::{Arena, Context, Key, KeyState, Window};
pub use winwin_common::{InternalEvent, KBDelta, WindowEvent};

#[link(name = "hooks.dll", kind = "dylib")]
extern "C" {
    fn init() -> Receiver<InternalEvent>;
}

#[link(name = "hooks.dll", kind = "dylib")]
extern "system" {
    fn cbt_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    fn low_level_keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
}

pub enum Event<'a> {
    KeyPress(Input<'a>),
    WindowOpen(Window),
    WindowClose(Window),
}

pub struct EventQueue {
    ev_rx: Receiver<InternalEvent>,
    key_map: KeyMap,

    _unsend: PhantomData<*const ()>,
    _unsync: PhantomData<*const ()>,
}

impl EventQueue {
    pub fn new() -> Self {
        let rx = unsafe { init() };

        thread::spawn(|| install_hooks());

        Self {
            ev_rx: rx,
            key_map: KeyMap::default(),

            _unsend: PhantomData,
            _unsync: PhantomData,
        }
    }

    pub fn next_event<'a>(&mut self, ctx: &'a Context) -> Event<'a> {
        ctx.arena.reset();

        let ev = self.ev_rx.recv().unwrap();
        match ev {
            InternalEvent::Keyboard(kb_delta, intercept_tx) => {
                self.key_map.update(kb_delta);
                let input = self.key_map.input(ctx, intercept_tx);
                Event::KeyPress(input)
            }
            InternalEvent::Shell(kind, handle) => {
                println!("hell event");
                let window = Window { handle };
                match kind {
                    WindowEvent::Created => Event::WindowOpen(window),
                    WindowEvent::Destroyed => Event::WindowClose(window),
                }
            }
        }
    }
}

fn install_hooks() {
    unsafe {
        create_hidden_window();
        let dll_name = s!("hooks.dll");
        let h_instance = GetModuleHandleA(dll_name)
            .expect("loading handle to current module should always succeed");

        let _ = SetWindowsHookExA(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), h_instance, 0);
        let _ = SetWindowsHookExA(WH_SHELL, Some(cbt_proc), h_instance, 0).unwrap();

        let _ = GetMessageA(std::ptr::null_mut(), None, 0, 0);
    }
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

    fn input<'a>(&self, ctx: &'a Context, tx: SyncSender<bool>) -> Input<'a> {
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
    intercept_tx: SyncSender<bool>,
}

impl<'a> Drop for Input<'a> {
    fn drop(&mut self) {
        let _ = self.intercept_tx.try_send(false);
    }
}

impl Input<'_> {
    pub fn pressed(&self, key: Key) -> bool {
        let pressed = self.pressed_no_intercept(key);
        if pressed {
            let _ = self.intercept_tx.try_send(true);
        }
        pressed
    }

    pub fn pressed_no_intercept(&self, key: Key) -> bool {
        self.keys[0] == key
    }

    pub fn all_pressed(&self, keys: &[Key]) -> bool {
        let pressed = self.all_pressed_no_intercept(keys);
        if pressed {
            let _ = self.intercept_tx.try_send(true);
        }
        pressed
    }

    pub fn all_pressed_no_intercept(&self, keys: &[Key]) -> bool {
        // Make sure len is the same otherwise we might match different keybind.
        keys.iter().all(|it| self.keys.iter().any(|k| *k == *it)) && self.keys.len() == keys.len()
    }
}

unsafe fn create_hidden_window() -> HWND {
    let class_name = s!("hidden window");
    let h_instance = GetModuleHandleA(None).unwrap();

    use windows::Win32::UI::WindowsAndMessaging::DefWindowProcA;
    // let wnd_class = WNDCLASSA {
    //     style: WNDCLASS_STYLES(0),
    //     lpfnWndProc: Some(DefWindowProcA),
    //     cbClsExtra: 0,
    //     cbWndExtra: 0,
    //     hInstance: h_instance.into(),
    //     hIcon: HICON(std::ptr::null_mut() as _),
    //     hCursor: HCURSOR(std::ptr::null_mut() as _),
    //     hbrBackground: (std::ptr::null_mut() as _),
    //     lpszMenuName: s!(""),
    //     lpszClassName: class_name,
    // };
    //
    // if RegisterClassA(&wnd_class) == 0 {
    //     panic!("Failed to register window class");
    // }

    let hwnd = CreateWindowExA(
        WINDOW_EX_STYLE(0),
        class_name,
        None,
        WINDOW_STYLE(0),
        0,
        0,
        0,
        0,
        None,
        None,
        h_instance,
        None,
    );

    hwnd
}
