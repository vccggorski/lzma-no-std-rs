//! lzma-rs fork containing only no_std based LZMA decoder (standalone function
//! & stream based)

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]
#![deny(missing_debug_implementations)]
#![deny(unsafe_code)]

#[macro_use]
mod macros;

mod decode;
#[cfg(feature = "std")]
mod encode;
pub mod error;

/// Module exposing `io` related traits and impls
pub mod io;

/// Compression helpers.
#[cfg(feature = "std")]
pub mod compress {
    pub use crate::encode::options::*;
}

/// Decompression helpers.
pub mod decompress {
    pub use crate::decode::options::*;
    #[cfg(feature = "stream")]
    pub use crate::decode::stream::Stream;
    #[cfg(feature = "stream")]
    pub use crate::decode::stream::StreamStatus;
}

/// Decompress LZMA data with default
/// [`Options`](decompress/struct.Options.html).
pub fn lzma_decompress<
    R: io::BufRead,
    W: io::Write,
    const DICT_MEM_LIMIT: usize,
    const PROBS_MEM_LIMIT: usize,
>(
    input: &mut R,
    output: &mut W,
) -> error::Result<()> {
    lzma_decompress_with_options::<_, _, DICT_MEM_LIMIT, PROBS_MEM_LIMIT>(
        input,
        output,
        &decompress::Options::default(),
    )
}

/// Decompress LZMA data with the provided options.
pub fn lzma_decompress_with_options<
    R: io::BufRead,
    W: io::Write,
    const DICT_MEM_LIMIT: usize,
    const PROBS_MEM_LIMIT: usize,
>(
    input: &mut R,
    output: &mut W,
    options: &decompress::Options,
) -> error::Result<()> {
    use crate::decode::lzbuffer::LzBuffer;
    use crate::decode::lzbuffer::LzCircularBuffer;
    let params = decode::lzma::LzmaParams::read_header(input, options)?;
    let mut decoder =
        decode::lzma::DecoderState::<LzCircularBuffer<DICT_MEM_LIMIT>, PROBS_MEM_LIMIT>::new();
    decoder.reset();
    decoder.set_params(params)?;

    let mut rangecoder = decode::rangecoder::RangeDecoder::new(input)
        .map_err(|_| error::lzma::LzmaError::DataStreamIsTooShort)?;
    decoder.process(output, &mut rangecoder)?;
    decoder.output.finish(output)?;
    Ok(())
}

/// Compresses data with LZMA and default
/// [`Options`](compress/struct.Options.html). Kept for tests
#[cfg(feature = "std")]
pub fn lzma_compress<R: io::BufRead, W: io::Write>(
    input: &mut R,
    output: &mut W,
) -> io::Result<()> {
    lzma_compress_with_options(input, output, &compress::Options::default())
}

/// Compress LZMA data with the provided options.
/// Kept for tests
#[cfg(feature = "std")]
pub fn lzma_compress_with_options<R: io::BufRead, W: io::Write>(
    input: &mut R,
    output: &mut W,
    options: &compress::Options,
) -> io::Result<()> {
    let encoder = encode::dumbencoder::Encoder::from_stream(output, options)?;
    encoder.process(input)
}

#[allow(missing_docs)]
/// Module containing alternative [`Option`] type implementation
pub mod option {
    /// Custom Option type guaranteed to have a proper variant ordering. This
    /// allows to achieve guaranteed 0-initializable
    /// [`crate::decompress::Stream`] with `Option::None` variant being 0
    #[repr(C)]
    #[derive(PartialEq, PartialOrd, Eq, Ord, Debug, Hash)]
    pub enum GuaranteedOption<T> {
        /// No value
        None,
        /// Some value `T`
        Some(T),
    }

    impl<T> GuaranteedOption<T> {
        pub const fn as_ref(&self) -> GuaranteedOption<&T> {
            use GuaranteedOption::*;
            match self {
                Some(v) => Some(v),
                None => None,
            }
        }
        pub fn as_mut(&mut self) -> GuaranteedOption<&mut T> {
            use GuaranteedOption::*;
            match self {
                Some(v) => Some(v),
                None => None,
            }
        }
        pub fn take(&mut self) -> Self {
            use GuaranteedOption::*;
            core::mem::replace(self, None)
        }
        pub fn replace(&mut self, value: T) -> Self {
            use GuaranteedOption::*;
            core::mem::replace(self, Some(value))
        }
    }

    impl<T: Clone> Clone for GuaranteedOption<T> {
        fn clone(&self) -> Self {
            use GuaranteedOption::*;
            match self {
                Some(x) => Some(x.clone()),
                None => None,
            }
        }
    }

    impl<T: Copy> Copy for GuaranteedOption<T> {}

    impl<T> From<Option<T>> for GuaranteedOption<T> {
        fn from(v: Option<T>) -> Self {
            match v {
                Some(v) => GuaranteedOption::Some(v),
                None => GuaranteedOption::None,
            }
        }
    }

    impl<T> From<GuaranteedOption<T>> for Option<T> {
        fn from(v: GuaranteedOption<T>) -> Self {
            match v {
                GuaranteedOption::Some(v) => Some(v),
                GuaranteedOption::None => None,
            }
        }
    }
}
