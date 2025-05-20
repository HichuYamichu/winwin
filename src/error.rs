use windows::core::HRESULT;

use crate::types::*;

pub const ERROR_INVALID_WINDOW_HANDLE: HRESULT = HRESULT::from_win32(0x578);
pub const ERROR_INVALID_HANDLE: HRESULT = HRESULT::from_win32(0x6);

#[derive(Debug)]
pub enum Error {
    BadWindow(Window),
    BadWindowHandle(WindowHandle),
    BadMonitor(Monitor),
    BadMonitorHandle(MonitorHandle),
    Windows(windows::core::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadWindow(e) => write!(f, "bad window entity: {:?}", e),
            Error::BadWindowHandle(h) => write!(f, "invalidated WindowHandle used: {:?}", h),
            Error::BadMonitor(e) => write!(f, "bad monitor entity: {:?}", e),
            Error::BadMonitorHandle(h) => write!(f, "invalidated WindowHandle used: {:?}", h),
            Error::Windows(e) => write!(f, "unexpected Windows error: {}", e),
        }
    }
}

#[derive(Debug)]
pub struct InvalidEntity;
