use crate::error;
use crate::io;
use heapless::Vec;

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
    fn append_literal(&mut self, stream: &mut W, lit: u8) -> error::Result<()>;
    // Fetch an LZ sequence (length, distance) from inside the buffer
    fn append_lz(&mut self, stream: &mut W, len: usize, dist: usize) -> error::Result<()>;
    // Consumes this buffer and flushes any data
    fn finish(self, stream: &mut W) -> io::Result<()>;
}

// A circular buffer for LZ sequences
pub struct LzCircularBuffer<const MEM_LIMIT: usize> {
    buf: Vec<u8, MEM_LIMIT>, // Circular buffer
    dict_size: usize,        // Length of the buffer
    cursor: usize,           // Current position
    len: usize,              // Total number of bytes sent through the buffer
}

impl<const MEM_LIMIT: usize> LzCircularBuffer<MEM_LIMIT> {
    pub fn new(dict_size: usize) -> Self {
        lzma_info!("Dict size in LZ buffer: {}", dict_size);
        Self {
            buf: Vec::new(),
            dict_size,
            cursor: 0,
            len: 0,
        }
    }

    fn get(&self, index: usize) -> u8 {
        *self.buf.get(index).unwrap_or(&0)
    }

    fn set(&mut self, index: usize, value: u8) -> error::Result<()> {
        let new_len = index + 1;

        if self.buf.len() < new_len {
            if new_len <= MEM_LIMIT {
                self.buf
                    .resize(new_len, 0)
                    .unwrap_or_else(|_| unreachable!());
            } else {
                return Err(error::Error::LzmaError(
                    "exceeded memory limit of {MEM_LIMIT}",
                ));
            }
        }
        self.buf[index] = value;
        Ok(())
    }
}

impl<W, const MEM_LIMIT: usize> LzBuffer<W> for LzCircularBuffer<MEM_LIMIT>
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
    fn append_literal(&mut self, stream: &mut W, lit: u8) -> error::Result<()> {
        self.set(self.cursor, lit)?;
        self.cursor += 1;
        self.len += 1;

        // Flush the circular buffer to the output
        if self.cursor == self.dict_size {
            stream.write_all(self.buf.as_slice())?;
            self.cursor = 0;
        }

        Ok(())
    }

    // Fetch an LZ sequence (length, distance) from inside the buffer
    fn append_lz(&mut self, stream: &mut W, len: usize, dist: usize) -> error::Result<()> {
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
            self.append_literal(stream, x)?;
            offset += 1;
            if offset == self.dict_size {
                offset = 0
            }
        }
        Ok(())
    }

    // Consumes this buffer and flushes any data
    fn finish(mut self, stream: &mut W) -> io::Result<()> {
        if self.cursor > 0 {
            stream.write_all(&self.buf[0..self.cursor])?;
            stream.flush()?;
        }
        Ok(())
    }
}
