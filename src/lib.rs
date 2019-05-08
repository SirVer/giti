#![feature(await_macro, async_await, futures_api)]

pub mod diffbase;
pub mod dispatch;
pub mod error;
pub mod git;
mod github;

pub use crate::diffbase::Diffbase;
pub use crate::error::Error;
pub use crate::error::ErrorKind;
pub use crate::error::Result;
