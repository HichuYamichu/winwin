use std::sync::mpsc::SyncSender;
use std::sync::OnceLock;
use serde::{Serialize, Deserialize};

use windows::Win32::Foundation::*;

mod keys;
pub use keys::*;

#[derive(Debug, Serialize, Deserialize)]
pub enum ServerCommand {
    InterceptKeypress,
    None
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ClientEvent {
    CBT(usize, WindowEventKind),
    Keyboard(KBDelta),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum WindowEventKind {
    Created,
    Destroyed,
}

#[derive(Debug, Serialize, Deserialize)]
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

#[macro_export]
macro_rules! log {
    ($logger:expr, $($arg:tt)*) => {
        $logger.log(format_args!($($arg)*));
    };
}

pub fn init_logger() -> Logger {
    Logger::new("C:\\winwin\\winwin.log")
}

pub static LOGGER: OnceLock<Logger> = OnceLock::new();

