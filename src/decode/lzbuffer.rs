use crate::error;
use crate::io;
use heapless::Vec;

pub trait LzBuffer<W>
where
    W: io::Write,
{
    fn set_dict_size(&mut self, dict_size: usize) -> error::Result<()>;
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
    fn finish(&mut self, stream: &mut W) -> io::Result<()>;
    fn reset(&mut self);
}

// TODO: Consider moving to raw array? Last heapless dependency I believe

// A circular buffer for LZ sequences
pub struct LzCircularBuffer<const MEM_LIMIT: usize> {
    buf: Vec<u8, MEM_LIMIT>,  // Circular buffer
    dict_size: Option<usize>, // Length of the buffer
    cursor: usize,            // Current position
    len: usize,               // Total number of bytes sent through the buffer
}

impl<const MEM_LIMIT: usize> LzCircularBuffer<MEM_LIMIT> {
    pub fn new() -> Self {
        let mut buf = Vec::new();
        buf.resize_default(MEM_LIMIT)
            .unwrap_or_else(|_| unreachable!("Buffer must be at least MEM_LIMIT"));
        Self {
            buf,
            dict_size: None,
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
            return Err(error::Error::LzmaError(
                "exceeded memory limit of {MEM_LIMIT}",
            ));
        }
        self.buf[index] = value;
        Ok(())
    }
}

impl<W, const MEM_LIMIT: usize> LzBuffer<W> for LzCircularBuffer<MEM_LIMIT>
where
    W: io::Write,
{
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
        // TODO: resolve optional dict_size in a different way
        let dict_size = self
            .dict_size
            .unwrap_or_else(|| panic!("LzCircularBuffer::dict_size is not initialized"));
        if self.len == 0 {
            lit
        } else {
            self.get((dict_size + self.cursor - 1) % dict_size)
        }
    }

    // Retrieve the n-th last byte
    fn last_n(&self, dist: usize) -> error::Result<u8> {
        let dict_size = self
            .dict_size
            .unwrap_or_else(|| panic!("LzCircularBuffer::dict_size is not initialized"));
        if dist > dict_size {
            return Err(error::Error::LzmaError(
                "Match distance {dist} is beyond dictionary size {dict_size}",
            ));
        }
        if dist > self.len {
            return Err(error::Error::LzmaError(
                "Match distance {dist} is beyond output size {self.len}",
            ));
        }

        let offset = (dict_size + self.cursor - dist) % dict_size;
        Ok(self.get(offset))
    }

    // Append a literal
    fn append_literal(&mut self, stream: &mut W, lit: u8) -> error::Result<()> {
        let dict_size = self
            .dict_size
            .unwrap_or_else(|| panic!("LzCircularBuffer::dict_size is not initialized"));
        self.set(self.cursor, lit)?;
        self.cursor += 1;
        self.len += 1;

        // Flush the circular buffer to the output
        if self.cursor == dict_size {
            stream.write_all(self.buf.as_slice())?;
            self.cursor = 0;
        }

        Ok(())
    }

    // Fetch an LZ sequence (length, distance) from inside the buffer
    fn append_lz(&mut self, stream: &mut W, len: usize, dist: usize) -> error::Result<()> {
        let dict_size = self
            .dict_size
            .unwrap_or_else(|| panic!("LzCircularBuffer::dict_size is not initialized"));
        lzma_debug!("LZ {{ len: {}, dist: {} }}", len, dist);
        if dist > dict_size {
            return Err(error::Error::LzmaError(
                "LZ distance {dist} is beyond dictionary size {dict_size}",
            ));
        }
        if dist > self.len {
            return Err(error::Error::LzmaError(
                "LZ distance {dist} is beyond output size {self.len}",
            ));
        }

        let mut offset = (dict_size + self.cursor - dist) % dict_size;
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
    fn finish(&mut self, stream: &mut W) -> io::Result<()> {
        if self.cursor > 0 {
            stream.write_all(&self.buf[0..self.cursor])?;
            stream.flush()?;
        }
        LzBuffer::<W>::reset(self);
        Ok(())
    }

    fn reset(&mut self) {
        self.buf.truncate(0);
        self.dict_size = None;
        self.cursor = 0;
        self.len = 0;
    }
}
