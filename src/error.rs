//! Error handling.

use crate::io;
use core::result;

pub mod lzma {
    #[derive(Debug)]
    pub enum LzmaError {
        MatchDistanceIsBeyondOutputSize { distance: usize, buffer_len: usize },
        ExceededMemoryLimit { memory_limit: usize },
    }
}

/// Library errors.
#[derive(Debug)]
pub enum Error {
    DictionaryBufferTooSmall {
        needed: usize,
        available: usize,
    },
    ProbabilitiesBufferTooSmall {
        needed: usize,
        available: usize,
    },
    /// I/O error.
    IoError(io::Error),
    /// Not enough bytes to complete header
    HeaderTooShort(io::Error),
    /// LZMA error.
    LzmaError(&'static str),
}

/// Library result alias.
pub type Result<T> = result::Result<T, Error>;

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::IoError(e)
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
