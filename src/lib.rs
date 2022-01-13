//! Pure-Rust codecs for LZMA, LZMA2, and XZ.

#![cfg_attr(feature = "no_std", no_std)]
#![deny(missing_debug_implementations)]

#[macro_use]
mod macros;

mod decode;
pub mod error;

use crate::allocator::MemoryDispenser;
use crate::decode::lzbuffer::LzBuffer;
use std::io;

/// Decompression helpers.
pub mod decompress {
    pub use crate::decode::options::*;
    #[cfg(feature = "stream")]
    pub use crate::decode::stream::Stream;
}

/// Decompress LZMA data with default
/// [`Options`](decompress/struct.Options.html).
pub fn lzma_decompress<'a, R: io::BufRead, W: io::Write>(
    mm: &MemoryDispenser<'a>,
    input: &mut R,
    output: &mut W,
) -> error::Result<()> {
    lzma_decompress_with_options(mm, input, output, &decompress::Options::default())
}

/// Decompress LZMA data with the provided options.
pub fn lzma_decompress_with_options<'a, R: io::BufRead, W: io::Write>(
    mm: &MemoryDispenser<'a>,
    input: &mut R,
    output: &mut W,
    options: &decompress::Options,
) -> error::Result<()> {
    let params = decode::lzma::LzmaParams::read_header(input, options)?;
    let mut decoder = if let Some(memlimit) = options.memlimit {
        decode::lzma::new_circular_with_memlimit(mm, output, params, memlimit)?
    } else {
        decode::lzma::new_circular(mm, output, params)?
    };

    let mut rangecoder = decode::rangecoder::RangeDecoder::new(input)
        .map_err(|e| error::Error::LzmaError("LZMA stream too short: {e}"))?;
    decoder.process(&mut rangecoder)?;
    decoder.output.finish()?;
    Ok(())
}

pub mod allocator {
    use core::cell::RefCell;

    #[derive(Debug)]
    pub struct MemoryDispenser<'a> {
        memory: &'a mut [u8],
        used: RefCell<usize>,
    }

    #[derive(Debug)]
    pub struct OutOfMemory {
        memory_size: usize,
        free_memory_left: usize,
        tried_to_allocate: usize,
    }

    impl<'a> MemoryDispenser<'a> {
        pub fn new(slice: &'a mut [u8]) -> Self {
            Self {
                memory: slice,
                used: 0_usize.into(),
            }
        }

        pub fn allocate<T, F: Fn() -> Result<T, OutOfMemory>>(
            &self,
            count: usize,
            init: F,
        ) -> Result<&mut [T], OutOfMemory> {
            let t_size = core::mem::size_of::<T>();
            let allocate_bytes = t_size * count;
            let used = *self.used.borrow();
            if used + allocate_bytes > self.memory.len() {
                return Err(OutOfMemory {
                    memory_size: self.memory.len(),
                    free_memory_left: self.memory.len() - used,
                    tried_to_allocate: allocate_bytes,
                });
            }
            let output_slice = unsafe {
                core::slice::from_raw_parts_mut(self.memory.as_ptr().add(used) as *mut T, count)
            };
            *self.used.borrow_mut() += allocate_bytes;
            for v in output_slice.iter_mut() {
                *v = init()?;
            }
            Ok(output_slice)
        }

        pub fn allocate_default<T: Default>(&self, count: usize) -> Result<&mut [T], OutOfMemory> {
            self.allocate(count, || Ok(Default::default()))
        }
    }
}
