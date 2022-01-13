//! Error handling.

use core::fmt::Display;
use core2::io;
use core::result;
use crate::allocator;

pub mod lzma {
    #[derive(Debug)]
    pub enum LzmaError {
        MatchDistanceIsBeyondOutputSize { distance: usize, buffer_len: usize },
        ExceededMemoryLimit { memory_limit: usize },
    }
}

pub mod xz {
    #[derive(Debug)]
    pub enum XzError {
        SomeError,
    }
}

/// Library errors.
#[derive(Debug)]
pub enum Error {
    OutOfMemory(allocator::OutOfMemory),
    /// I/O error.
    IoError(io::Error),
    /// Not enough bytes to complete header
    HeaderTooShort(io::Error),
    /// LZMA error.
    LzmaError(&'static str),
    /// XZ error.
    XzError(&'static str),
}

/// Library result alias.
pub type Result<T> = result::Result<T, Error>;

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::IoError(e)
    }
}

impl From<allocator::OutOfMemory> for Error {
    fn from(e: allocator::OutOfMemory) -> Self {
        Error::OutOfMemory(e)
    }
}

impl Display for Error {
    fn fmt(&self, fmt: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::OutOfMemory(e) => write!(fmt, "oom error: {:?}", e),
            Error::IoError(e) => write!(fmt, "io error: {}", e),
            Error::HeaderTooShort(e) => write!(fmt, "header too short: {}", e),
            Error::LzmaError(e) => write!(fmt, "lzma error: {:?}", e),
            Error::XzError(e) => write!(fmt, "xz error: {:?}", e),
        }
    }
}

#[cfg(test)]
mod test {
    use super::Error;

    #[test]
    fn test_display() {
        assert_eq!(
            Error::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "this is an error"
            ))
            .to_string(),
            "io error: this is an error"
        );
        assert_eq!(
            Error::LzmaError("this is an error".to_string()).to_string(),
            "lzma error: this is an error"
        );
        assert_eq!(
            Error::XzError("this is an error".to_string()).to_string(),
            "xz error: this is an error"
        );
    }
}
