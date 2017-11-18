use std::error;
use std::fmt;
use std::result;

#[derive(Debug, PartialEq)]
pub enum ErrorKind {
    GeneralError,
    SubcommandFailed,
}

#[derive(Debug)]
pub struct Error {
    pub description: String,
    pub kind: ErrorKind,
}

pub type Result<T> = result::Result<T, Error>;

impl Error {
    pub fn general(s: String) -> Error {
        Error {
            description: s,
            kind: ErrorKind::GeneralError,
        }
    }

    pub fn subcommand_fail(command: &str, code: i32) -> Error {
        Error {
            description: format!("{} exited with {}", command, code),
            kind: ErrorKind::SubcommandFailed,
        }
    }

    pub fn description(&self) -> &str {
        &self.description
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "{}", self.description)
    }
}

impl<T: error::Error> From<T> for Error {
    fn from(err: T) -> Error {
        Error::general(err.description().to_string())
    }
}
