use windows::core::HRESULT;

use crate::types::*;

pub const ERROR_INVALID_WINDOW_HANDLE: HRESULT = HRESULT(0x80070578u32 as i32);

#[derive(Debug)]
pub enum Error {
    BadWindowHandle(WindowHandle),
    Windows(windows::core::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Windows(e) => write!(f, "unexpected Windows error: {}", e),
            Error::BadWindowHandle(h) => write!(f, "invalidated WindowHandle used: {:?}", h),
        }
    }
}

#[derive(Debug)]
pub struct InvalidEntity;
