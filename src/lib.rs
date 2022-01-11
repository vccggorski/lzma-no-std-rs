//! Pure-Rust codecs for LZMA, LZMA2, and XZ.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_debug_implementations)]

#[macro_use]
mod macros;

mod decode;
pub mod error;
mod io_ext;

use crate::allocator::Allocator;
use crate::decode::lzbuffer::LzBuffer;
pub use core2::io;

/// Decompression helpers.
pub mod decompress {
    pub use crate::decode::options::*;
    #[cfg(feature = "stream")]
    pub use crate::decode::stream::Stream;
}

/// Decompress LZMA data with default
/// [`Options`](decompress/struct.Options.html).
pub fn lzma_decompress<A: Allocator, R: io::BufRead, W: io::Write>(
    mm: &A,
    input: &mut R,
    output: &mut W,
) -> error::Result<()>
where
    error::Error: From<A::Error>,
{
    lzma_decompress_with_options(mm, input, output, &decompress::Options::default())
}

/// Decompress LZMA data with the provided options.
pub fn lzma_decompress_with_options<A: Allocator, R: io::BufRead, W: io::Write>(
    mm: &A,
    input: &mut R,
    output: &mut W,
    options: &decompress::Options,
) -> error::Result<()>
where
    error::Error: From<A::Error>,
{
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

    pub unsafe trait Allocator {
        type Error;

        fn allocate<T: Allocatable, F: Fn() -> Result<T, Self::Error>>(
            &self,
            count: usize,
            init: F,
        ) -> Result<&mut [T], Self::Error>;

        fn allocate_default<T: Allocatable + Default>(
            &self,
            count: usize,
        ) -> Result<&mut [T], Self::Error> {
            self.allocate(count, || Ok(Default::default()))
        }
    }

    /// This trait prevents users from allocating types that might require
    /// non-trivial destruction (eg. freeing allocated memory) and should be
    /// only implemented for types that are either [`Copy`] or field thereof
    /// points to other [`Allocatable`] type.
    pub unsafe trait Allocatable {}
    unsafe impl Allocatable for u8 {}
    unsafe impl Allocatable for u16 {}
    unsafe impl<T: Copy, const S: usize> Allocatable for heapless::Vec<T, S> {}
    unsafe impl<'a> Allocatable for crate::decode::rangecoder::BitTree<'a> {}

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
    }

    unsafe impl<'a> Allocator for MemoryDispenser<'a> {
        type Error = OutOfMemory;
        fn allocate<T: Allocatable, F: Fn() -> Result<T, Self::Error>>(
            &self,
            count: usize,
            init: F,
        ) -> Result<&mut [T], Self::Error> {
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
    }

    #[cfg(feature = "std")]
    use core::any::Any;
    #[cfg(feature = "std")]
    use core::convert::Infallible;
    #[cfg(feature = "std")]
    use std::vec::Vec;

    #[cfg(feature = "std")]
    #[derive(Debug, Default)]
    pub struct StdMemoryDispenser {
        memory: RefCell<Vec<Vec<u8>>>,
    }

    #[cfg(feature = "std")]
    unsafe impl Allocator for StdMemoryDispenser {
        type Error = Infallible;
        fn allocate<T: Allocatable, F: Fn() -> Result<T, Self::Error>>(
            &self,
            count: usize,
            init: F,
        ) -> Result<&mut [T], Self::Error> {
            use core::convert::TryInto;
            use core::iter::{repeat_with, FromIterator};
            let t_size = core::mem::size_of::<T>();
            let allocate_bytes = t_size * count;
            self.memory.borrow_mut().push(vec![0_u8; allocate_bytes]);
            let output_slice = unsafe {
                core::slice::from_raw_parts_mut(
                    self.memory.borrow().last().unwrap_unchecked().as_ptr() as *mut T,
                    count,
                )
            };
            for v in output_slice.iter_mut() {
                *v = init()?;
            }
            Ok(output_slice)
        }
    }
}
