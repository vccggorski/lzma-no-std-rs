use crate::allocator::Allocator;
use crate::error;
use core2::io;

pub trait LzBuffer<W>
where
    W: io::Write,
{
    fn len(&self) -> usize;
    // Retrieve the last byte or return a default
    fn last_or(&self, lit: u8) -> u8;
    // Retrieve the n-th last byte
    fn last_n(&self, dist: usize) -> error::Result<u8>;
    // Append a literal
    fn append_literal(&mut self, lit: u8) -> error::Result<()>;
    // Fetch an LZ sequence (length, distance) from inside the buffer
    fn append_lz(&mut self, len: usize, dist: usize) -> error::Result<()>;
    // Get a reference to the output sink
    fn get_output(&self) -> &W;
    // Get a mutable reference to the output sink
    fn get_output_mut(&mut self) -> &mut W;
    // Consumes this buffer and flushes any data
    fn finish(self) -> io::Result<W>;
    // Consumes this buffer without flushing any data
    fn into_output(self) -> W;
}

// A circular buffer for LZ sequences
pub struct LzCircularBuffer<'a, W>
where
    W: io::Write,
{
    stream: W,         // Output sink
    buf: &'a mut [u8], // Circular buffer
    dict_size: usize,  // Length of the buffer
    cursor: usize,     // Current position
    len: usize,        // Total number of bytes sent through the buffer
}

impl<'a, W> LzCircularBuffer<'a, W>
where
    W: io::Write,
{
    pub fn from_stream_with_memlimit<A: Allocator>(
        mm: &'a A,
        stream: W,
        dict_size: usize,
        memlimit: usize,
    ) -> Result<Self, A::Error> {
        lzma_info!("Dict size in LZ buffer: {}", dict_size);
        Ok(Self {
            stream,
            buf: mm.allocate_default(memlimit)?,
            dict_size,
            cursor: 0,
            len: 0,
        })
    }

    fn get(&self, index: usize) -> u8 {
        *self.buf.get(index).unwrap_or(&0)
    }

    fn set(&mut self, index: usize, value: u8) -> error::Result<()> {
        if self.buf.len() <= index + 1 {
            return Err(error::Error::LzmaError(
                "exceeded memory limit of {self.memlimit}",
            ));
        }
        self.buf[index] = value;
        Ok(())
    }
}

impl<'a, W> LzBuffer<W> for LzCircularBuffer<'a, W>
where
    W: io::Write,
{
    fn len(&self) -> usize {
        self.len
    }

    // Retrieve the last byte or return a default
    fn last_or(&self, lit: u8) -> u8 {
        if self.len == 0 {
            lit
        } else {
            self.get((self.dict_size + self.cursor - 1) % self.dict_size)
        }
    }

    // Retrieve the n-th last byte
    fn last_n(&self, dist: usize) -> error::Result<u8> {
        if dist > self.dict_size {
            return Err(error::Error::LzmaError(
                "Match distance {dist} is beyond dictionary size {self.dict_size}",
            ));
        }
        if dist > self.len {
            return Err(error::Error::LzmaError(
                "Match distance {dist} is beyond output size {self.len}",
            ));
        }

        let offset = (self.dict_size + self.cursor - dist) % self.dict_size;
        Ok(self.get(offset))
    }

    // Append a literal
    fn append_literal(&mut self, lit: u8) -> error::Result<()> {
        self.set(self.cursor, lit)?;
        self.cursor += 1;
        self.len += 1;

        // Flush the circular buffer to the output
        if self.cursor == self.dict_size {
            self.stream.write_all(self.buf)?;
            self.cursor = 0;
        }

        Ok(())
    }

    // Fetch an LZ sequence (length, distance) from inside the buffer
    fn append_lz(&mut self, len: usize, dist: usize) -> error::Result<()> {
        lzma_debug!("LZ {{ len: {}, dist: {} }}", len, dist);
        if dist > self.dict_size {
            return Err(error::Error::LzmaError(
                "LZ distance {dist} is beyond dictionary size {self.dict_size}",
            ));
        }
        if dist > self.len {
            return Err(error::Error::LzmaError(
                "LZ distance {dist} is beyond output size {self.len}",
            ));
        }

        let mut offset = (self.dict_size + self.cursor - dist) % self.dict_size;
        for _ in 0..len {
            let x = self.get(offset);
            self.append_literal(x)?;
            offset += 1;
            if offset == self.dict_size {
                offset = 0
            }
        }
        Ok(())
    }

    // Get a reference to the output sink
    fn get_output(&self) -> &W {
        &self.stream
    }

    // Get a mutable reference to the output sink
    fn get_output_mut(&mut self) -> &mut W {
        &mut self.stream
    }

    // Consumes this buffer and flushes any data
    fn finish(mut self) -> io::Result<W> {
        if self.cursor > 0 {
            self.stream.write_all(&self.buf[0..self.cursor])?;
            self.stream.flush()?;
        }
        Ok(self.stream)
    }

    // Consumes this buffer without flushing any data
    fn into_output(self) -> W {
        self.stream
    }
}
