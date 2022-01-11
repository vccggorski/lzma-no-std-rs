//! Error handling.
#![allow(missing_docs)]

use crate::io;
use core::result;

pub mod lzma {
    #[derive(PartialEq, Debug)]
    pub enum LzmaError {
        MatchDistanceIsBeyondDictionarySize {
            distance: usize,
            dict_size: usize,
        },
        MatchDistanceIsBeyondOutputSize {
            distance: usize,
            output_len: usize,
        },
        LzDistanceIsBeyondDictionarySize {
            distance: usize,
            dict_size: usize,
        },
        LzDistanceIsBeyondOutputSize {
            distance: usize,
            output_len: usize,
        },
        /// `properties` must be < 255
        InvalidHeader {
            invalid_properties: u32,
        },
        EosFoundButMoreBytesAvailable,
        ProcessedDataDoesNotMatchUnpackedSize {
            unpacked_size: u64,
            decompressed_data: usize,
        },
        /// When processing is done in `Finish`, standalone mode and `RangeDecoder` 
        DataStreamIsTooShort,
    }
}

pub mod stream {
    #[derive(PartialEq, Debug)]
    pub enum StreamError {
        /// When `finish` is called and header parsing was never completed
        FailedToReadLzmaHeader,
        /// When `finish` is called but previous errors corrupted the stream
        /// state
        InvalidState,
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
    LzmaError(lzma::LzmaError),
    StreamError(stream::StreamError),
}

/// Library result alias.
pub type Result<T> = result::Result<T, Error>;

impl From<lzma::LzmaError> for Error {
    fn from(e: lzma::LzmaError) -> Self {
        Error::LzmaError(e)
    }
}

impl From<stream::StreamError> for Error {
    fn from(e: stream::StreamError) -> Self {
        Error::StreamError(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::IoError(e)
    }
}
