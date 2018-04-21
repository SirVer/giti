#![feature(proc_macro, generators)]

extern crate futures_await as futures;
extern crate git2;
extern crate hubcaps;
extern crate hyper;
extern crate hyper_tls;
extern crate term;
extern crate tokio_core;

pub mod dispatch;
pub mod error;
pub mod git;
mod github;

pub use error::Error;
pub use error::ErrorKind;
pub use error::Result;
