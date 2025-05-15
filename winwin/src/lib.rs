#![feature(allocator_api)]
#![feature(get_mut_unchecked)]

pub mod error;
pub mod events;
pub mod input;
pub mod types;
pub mod wm;

pub mod context;
pub use context::*;
pub use events::WindowManager;

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
