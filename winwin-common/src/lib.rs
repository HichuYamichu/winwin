use std::sync::mpsc::SyncSender;

use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

mod keys;
pub use keys::*;

pub enum InternalEvent {
    Keyboard(KBDelta, SyncSender<bool>),
    Shell(WindowEvent, HWND),
}

pub enum WindowEvent {
    Created,
    Destroyed,
}

pub struct KBDelta {
    pub vk_code: u8,
    pub key_state: KeyState,
}

use std::fmt::Arguments;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;

pub struct Logger {
    file: Mutex<std::fs::File>,
}

impl Logger {
    pub fn new(log_file: &str) -> Logger {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file)
            .expect("Unable to open log file");

        Logger {
            file: Mutex::new(file),
        }
    }

    pub fn log(&self, args: Arguments) {
        let mut file = self.file.lock().unwrap();
        writeln!(file, "{}", args).expect("Unable to write to log file");
    }
}

use std::sync::OnceLock;

#[macro_export]
macro_rules! log {
    ($logger:expr, $($arg:tt)*) => {
        $logger.log(format_args!($($arg)*));
    };
}

