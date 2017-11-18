extern crate git2;
extern crate term;

pub mod dispatch;
pub mod error;
pub mod git;

pub use error::Error;
pub use error::ErrorKind;
pub use error::Result;
