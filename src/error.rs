//! Error handling.

use crate::io;
use core::result;

pub mod lzma {
    #[derive(PartialEq, Debug)]
    pub enum LzmaError {
        MatchDistanceIsBeyondDictionarySize { distance: usize, dict_size: usize },
        MatchDistanceIsBeyondOutputSize { distance: usize, buffer_len: usize },
        LzDistanceIsBeyondDictionarySize { distance: usize, dict_size: usize },
        LzDistanceIsBeyondOutputSize { distance: usize, buffer_len: usize },
        InvalidHeader,
        EosFoundButMoreBytesAvailable,
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
