#![warn(unsafe_code)]
use crate::error;
use crate::io;
use crate::option::GuaranteedOption as Option;
use crate::option::GuaranteedOption::*;

pub trait LzBuffer {
    fn set_dict_size(&mut self, dict_size: usize) -> error::Result<()>;
    fn len(&self) -> usize;
    // Retrieve the last byte or return a default
    fn last_or(&self, lit: u8) -> u8;
    // Retrieve the n-th last byte
    fn last_n(&self, dist: usize) -> error::Result<u8>;
    // Append a literal
    fn append_literal(&mut self, stream: &mut dyn io::Write, lit: u8) -> error::Result<()>;
    // Fetch an LZ sequence (length, distance) from inside the buffer
    fn append_lz(
        &mut self,
        stream: &mut dyn io::Write,
        len: usize,
        dist: usize,
    ) -> error::Result<()>;
    // Consumes this buffer and flushes any data
    fn finish(&mut self, stream: &mut dyn io::Write) -> io::Result<()>;
    fn reset(&mut self);
}

// A circular buffer for LZ sequences
pub struct LzCircularBuffer<const MEM_LIMIT: usize> {
    buf: [u8; MEM_LIMIT],     // Circular buffer
    dict_size: Option<usize>, // Length of the buffer
    cursor: usize,            // Current position
    len: usize,               // Total number of bytes sent through the buffer
}

impl<const MEM_LIMIT: usize> LzCircularBuffer<MEM_LIMIT> {
    pub const fn new() -> Self {
        Self {
            buf: [0_u8; MEM_LIMIT],
            dict_size: None,
            cursor: 0,
            len: 0,
        }
    }

    fn get(&self, index: usize) -> u8 {
        *self.buf.get(index).unwrap_or(&0)
    }

    fn set(&mut self, index: usize, value: u8) {
        self.buf[index] = value;
    }
}

impl<const MEM_LIMIT: usize> LzBuffer for LzCircularBuffer<MEM_LIMIT> {
    fn set_dict_size(&mut self, dict_size: usize) -> error::Result<()> {
        lzma_info!("Dict size in LZ buffer: {}", dict_size);
        if dict_size > MEM_LIMIT {
            return Err(error::Error::DictionaryBufferTooSmall {
                needed: dict_size,
                available: MEM_LIMIT,
            });
        }
        self.dict_size = Some(dict_size);
        Ok(())
    }

    fn len(&self) -> usize {
        self.len
    }

    // Retrieve the last byte or return a default
    fn last_or(&self, lit: u8) -> u8 {
        let dict_size = unsafe { self.dict_size.as_ref().unwrap_unchecked().clone() };
        if self.len == 0 {
            lit
        } else {
            self.get((dict_size + self.cursor - 1) % dict_size)
        }
    }

    // Retrieve the n-th last byte
    fn last_n(&self, distance: usize) -> error::Result<u8> {
        let dict_size = unsafe { self.dict_size.as_ref().unwrap_unchecked().clone() };
        if distance > dict_size {
            return Err(
                error::lzma::LzmaError::MatchDistanceIsBeyondDictionarySize {
                    distance,
                    dict_size,
                }
                .into(),
            );
        }
        if distance > self.len {
            return Err(error::lzma::LzmaError::MatchDistanceIsBeyondOutputSize {
                distance,
                output_len: self.len,
            }
            .into());
        }

        let offset = (dict_size + self.cursor - distance) % dict_size;
        Ok(self.get(offset))
    }

    // Append a literal
    fn append_literal(&mut self, stream: &mut dyn io::Write, lit: u8) -> error::Result<()> {
        let dict_size = unsafe { self.dict_size.as_ref().unwrap_unchecked().clone() };
        self.set(self.cursor, lit);
        self.cursor += 1;
        self.len += 1;

        // Flush the circular buffer to the output
        if self.cursor == dict_size {
            stream.write_all(&self.buf[..self.cursor])?;
            self.cursor = 0;
        }

        Ok(())
    }

    // Fetch an LZ sequence (length, distance) from inside the buffer
    fn append_lz(
        &mut self,
        stream: &mut dyn io::Write,
        len: usize,
        distance: usize,
    ) -> error::Result<()> {
        let dict_size = unsafe { self.dict_size.as_ref().unwrap_unchecked().clone() };
        lzma_debug!("LZ {{ len: {}, distance: {} }}", len, distance);
        if distance > dict_size {
            return Err(error::lzma::LzmaError::LzDistanceIsBeyondDictionarySize {
                distance,
                dict_size,
            }
            .into());
        }
        if distance > self.len {
            return Err(error::lzma::LzmaError::LzDistanceIsBeyondOutputSize {
                distance,
                output_len: self.len,
            }
            .into());
        }

        let mut offset = (dict_size + self.cursor - distance) % dict_size;
        for _ in 0..len {
            let x = self.get(offset);
            self.append_literal(stream, x)?;
            offset += 1;
            if offset == dict_size {
                offset = 0
            }
        }
        Ok(())
    }

    // Consumes this buffer and flushes any data
    fn finish(&mut self, stream: &mut dyn io::Write) -> io::Result<()> {
        if self.cursor > 0 {
            stream.write_all(&self.buf[..self.cursor])?;
            stream.flush()?;
        }
        self.reset();
        Ok(())
    }

    fn reset(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0);
        self.dict_size = None;
        self.cursor = 0;
        self.len = 0;
    }
}
