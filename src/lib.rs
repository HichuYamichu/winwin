#![feature(allocator_api)]
#![feature(get_mut_unchecked)]

pub mod error;
pub mod events;
pub mod input;
pub mod types;
pub mod wm;

pub use events::*;
pub use input::Key;
pub use types::*;

#[macro_export]
macro_rules! map_err {
    (
        $call:expr,
        $($code:pat_param => $mapped:expr),+ $(,)?
    ) => {{
        use crate::error::*;
        let result = $call;
        match result {
            Ok(val) => Ok(val),
            Err(e) => {
                let code = e.code();
                match code {
                    $($code => Err($mapped),)+
                    _ => Err(Error::Windows(e)),
                }
            }
        }?
    }};
}

#[macro_export]
macro_rules! map_err_b {
    (
        $call:expr,
        $($code:pat_param => $mapped:expr),+ $(,)?
    ) => {{
        use crate::error::*;
        use windows::Win32::Foundation::{GetLastError};
        use windows::core::Error as WinError;
        use windows::core::HRESULT ;

        let success = $call;
        if success.as_bool() {
            ()
        } else {
            let win32_err = GetLastError().0;
            let code = HRESULT::from_win32(win32_err);
            let e = WinError::from(code);
            match code {
                $($code => Err($mapped),)+
                _ => Err(Error::Windows(e)),
            }?
        }
    }};
}

#[macro_export]
macro_rules! try_or_bail {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(_) => continue,
        }
    };
}
