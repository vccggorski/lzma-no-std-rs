//! lzma-rs fork containing only no_std based LZMA decoder (standalone function & stream based)

#![no_std]
#![deny(missing_docs)]
#![deny(missing_debug_implementations)]
#![forbid(unsafe_code)]

#[macro_use]
mod macros;

mod io_ext;
mod decode;
pub mod error;


pub mod io {
    pub use core2::io::*;
    pub use crate::io_ext::*;
}

/// Decompression helpers.
pub mod decompress {
    pub use crate::decode::options::*;
    #[cfg(feature = "stream")]
    pub use crate::decode::stream::Stream;
}

/// Decompress LZMA data with default [`Options`](decompress/struct.Options.html).
pub fn lzma_decompress<R: io::BufRead, W: io::Write>(
    input: &mut R,
    output: &mut W,
) -> error::Result<()> {
    lzma_decompress_with_options(input, output, &decompress::Options::default())
}

/// Decompress LZMA data with the provided options.
pub fn lzma_decompress_with_options<R: io::BufRead, W: io::Write>(
    input: &mut R,
    output: &mut W,
    options: &decompress::Options,
) -> error::Result<()> {
    let params = decode::lzma::LzmaParams::read_header(input, options)?;
    let mut decoder = if let Some(memlimit) = options.memlimit {
        decode::lzma::new_circular_with_memlimit(output, params, memlimit)?
    } else {
        decode::lzma::new_circular(output, params)?
    };

    let mut rangecoder = decode::rangecoder::RangeDecoder::new(input)
        .map_err(|e| error::Error::LzmaError("LZMA stream too short: {e}"))?;
    decoder.process(&mut rangecoder)?;
    decoder.output.finish()?;
    Ok(())
}
