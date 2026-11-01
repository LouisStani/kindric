use std::fmt;
use std::io;

/// One-line message shown as `claude-accounts: <message>` before exiting with code 1.
#[derive(Debug, PartialEq, Eq)]
pub struct Error(pub String);

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self(error.to_string())
    }
}

pub fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}
