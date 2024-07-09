use allocator_api2::vec::*;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{mpsc, Mutex};
use std::{marker::PhantomData, thread};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::{Arena, Context, Key, Window, LOCK_WAS_POISONED_MSG};
use once_cell::sync::Lazy;

pub enum Event<'a> {
    KeyPress(Input<'a>),
    WindowOpen(Window),
    WindowClose(Window),
}

enum InternalEvent {
    Keyboard(KBDelta, SyncSender<bool>),
}

pub struct EventQueue {
    ev_rx: Receiver<InternalEvent>,
    key_map: KeyMap,

    _unsend: PhantomData<*const ()>,
    _unsync: PhantomData<*const ()>,
}

static mut INTERNAL_EVENT_SENDER: Option<SyncSender<InternalEvent>> = None;

impl EventQueue {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(0);
        thread::spawn(|| install_hooks(tx));

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
        }
    }
}

fn install_hooks(tx: SyncSender<InternalEvent>) {
    unsafe {
        INTERNAL_EVENT_SENDER = Some(tx);

        let h_instance =
            GetModuleHandleA(None).expect("loading handle to current module should always succeed");
        // TODO: Implement clanup.
        let _ = SetWindowsHookExA(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), h_instance, 0);

        let _ = GetMessageA(std::ptr::null_mut(), None, 0, 0);
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

struct KBDelta {
    vk_code: u8,
    key_state: KeyState,
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

impl From<WPARAM> for KeyState {
    fn from(wparam: WPARAM) -> Self {
        if wparam == WPARAM(WM_KEYDOWN as _) || wparam == WPARAM(WM_SYSKEYDOWN as _) {
            return KeyState::Down;
        }

        if wparam == WPARAM(WM_KEYUP as _) || wparam == WPARAM(WM_SYSKEYUP as _) {
            return KeyState::Up;
        }

        unreachable!();
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
        self.keys.iter().any(|it| *it == key)
    }

    pub fn any_pressed(&self, keys: &[Key]) -> bool {
        let pressed = self.any_pressed_no_intercept(keys);
        if pressed {
            let _ = self.intercept_tx.try_send(true);
        }
        pressed
    }

    pub fn any_pressed_no_intercept(&self, keys: &[Key]) -> bool {
        keys.iter().any(|it| self.pressed_no_intercept(*it))
    }

    pub fn all_pressed(&self, keys: &[Key]) -> bool {
        let pressed = self.all_pressed_no_intercept(keys);
        if pressed {
            let _ = self.intercept_tx.try_send(true);
        }
        pressed
    }

    pub fn all_pressed_no_intercept(&self, keys: &[Key]) -> bool {
        keys.iter().all(|it| self.pressed_no_intercept(*it))
    }
}
