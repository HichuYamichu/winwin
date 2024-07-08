use std::{marker::PhantomData, thread};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use std::sync::Mutex;
use allocator_api2::vec::*;

use crate::{Context, Arena, Window, Key, LOCK_WAS_POISONED_MSG};
use once_cell::sync::Lazy;

static KB_EVENT: Lazy<HANDLE> = Lazy::new(|| unsafe {
    CreateEventA(None, false, false, None).expect("it must be possible to create an event")
});

pub enum WMEvent<'a> {
    InputChange(Input<'a>),
    WindowOpen(Window),
    WindowClose(Window),
}

pub struct EventQueue {
    _unsend: PhantomData<*const ()>,
    _unsync: PhantomData<*const ()>,
}

impl EventQueue {
    // TODO: Allow for only one instance at a time.
    pub fn new() -> Self {
        unsafe {
            thread::spawn(|| {
                let h_instance = GetModuleHandleA(None)
                    .expect("loading handle to current module should always succeed");
                // TODO: Implement clanup.
                let _ =
                    SetWindowsHookExA(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), h_instance, 0);

                let _ = GetMessageA(std::ptr::null_mut(), None, 0, 0);
            });
            Self {
                _unsend: PhantomData,
                _unsync: PhantomData,
            }
        }
    }

    pub fn next_event<'a>(&self, ctx: &'a Context) -> WMEvent<'a> {
        ctx.arena.reset();

        unsafe {
            WaitForSingleObject(*KB_EVENT, INFINITE);
        }

        let input = KEY_MAP.input(ctx);
        WMEvent::InputChange(input)
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let kb_info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if wparam == WPARAM(WM_KEYDOWN as _) || wparam == WPARAM(WM_SYSKEYDOWN as _) {
            KEY_MAP.set(kb_info.vkCode as _, KeyState::Down);
            if SetEvent(*KB_EVENT).is_err() {
                eprintln!("{:?}", GetLastError());
            }
        }

        if wparam == WPARAM(WM_KEYUP as _) || wparam == WPARAM(WM_SYSKEYUP as _) {
            KEY_MAP.set(kb_info.vkCode as _, KeyState::Up);
            if SetEvent(*KB_EVENT).is_err() {
                eprintln!("{:?}", GetLastError());
            }
        }
    }

    return CallNextHookEx(None, code, wparam, lparam);
}

#[derive(Debug, Copy, Clone)]
pub enum KeyState {
    Up,
    Down,
}

impl From<KeyState> for bool {
    fn from(ks: KeyState) -> Self {
        match ks {
            KeyState::Up => false,
            KeyState::Down => true,
        }
    }
}

pub struct KeyMap {
    keys: Mutex<[bool; 255]>,
}

pub static KEY_MAP: KeyMap = KeyMap {
    keys: Mutex::new([false; 255]),
};

impl KeyMap {
    pub fn set(&self, key_code: u8, ks: KeyState) {
        let mut keys = self.keys.lock().expect(LOCK_WAS_POISONED_MSG);
        keys[key_code as usize] = bool::from(ks);
    }

    pub fn input<'a>(&self, ctx: &'a Context) -> Input<'a> {
        let mut v = Vec::new_in(&ctx.arena);
        let keys = self.keys.lock().expect(LOCK_WAS_POISONED_MSG);
        for key in keys
            .iter()
            .enumerate()
            .filter(|(_, pressed)| **pressed)
            .filter_map(|(idx, _)| Key::try_from_vk(idx as _).ok())
        {
            v.push(key);
        }
        Input { keys: v }
    }
}

#[derive(Debug)]
pub struct Input<'a> {
    keys: Vec<Key, &'a Arena>,
}

impl Input<'_> {
    pub fn pressed(&self, key: Key) -> bool {
        self.keys.iter().any(|it| *it == key)
    }

    pub fn any_pressed(&self, keys: &[Key]) -> bool {
        keys.iter().any(|it| self.pressed(*it))
    }

    pub fn all_pressed(&self, keys: &[Key]) -> bool {
        keys.iter().all(|it| self.pressed(*it))
    }
}
