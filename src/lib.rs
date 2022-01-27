//! lzma-rs fork containing only no_std based LZMA decoder (standalone function
//! & stream based)

#![no_std]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]
#![deny(unsafe_code)]

#[macro_use]
mod macros;

mod decode;
pub mod error;
mod io_ext;

pub mod io {
    pub use crate::io_ext::*;
    pub use core2::io::*;
}

/// Decompression helpers.
pub mod decompress {
    pub use crate::decode::options::*;
    #[cfg(feature = "stream")]
    pub use crate::decode::stream::Stream;
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
        decode::lzma::DecoderState::<_, LzCircularBuffer<_, DICT_MEM_LIMIT>, PROBS_MEM_LIMIT>::new(
            output, params,
        )?;

    let mut rangecoder = decode::rangecoder::RangeDecoder::new(input)
        .map_err(|e| error::Error::LzmaError("LZMA stream too short: {e}"))?;
    decoder.process(&mut rangecoder)?;
    decoder.output.finish()?;
    Ok(())
}
