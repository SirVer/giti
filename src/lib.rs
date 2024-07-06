pub mod diffbase;
pub mod dispatch;
pub mod error;
pub mod git;
mod github;
mod gitlab;

pub use crate::diffbase::Diffbase;
pub use crate::error::Error;
pub use crate::error::ErrorKind;
pub use crate::error::Result;
