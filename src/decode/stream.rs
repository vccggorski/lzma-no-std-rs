use crate::decode::lzbuffer::{LzBuffer, LzCircularBuffer};
use crate::decode::lzma::{DecoderState, LzmaParams};
use crate::decode::rangecoder::RangeDecoder;
use crate::decompress::Options;
use crate::error;
use crate::io::{self, BufRead, Cursor, Read, Write};
use core::fmt::Debug;

/// Minimum header length to be read.
/// - props: u8 (1 byte)
/// - dict_size: u32 (4 bytes)
const MIN_HEADER_LEN: usize = 5;

/// Max header length to be read.
/// - unpacked_size: u64 (8 bytes)
const MAX_HEADER_LEN: usize = MIN_HEADER_LEN + 8;

/// Required bytes after the header.
/// - ignore: u8 (1 byte)
/// - code: u32 (4 bytes)
const START_BYTES: usize = 5;

/// Maximum number of bytes to buffer while reading the header.
const MAX_TMP_LEN: usize = MAX_HEADER_LEN + START_BYTES;

/// Internal state of this streaming decoder. This is needed because we have to
/// initialize the stream before processing any data.
#[derive(Debug)]
enum State<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize> {
    /// Stream is initialized but header values have not yet been read.
    Header,
    /// Header values have been read and the stream is ready to process more
    /// data.
    Data(RunState<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>),
}

/// Structures needed while decoding data.
struct RunState<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize> {
    range: u32,
    code: u32,
}

impl<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize> Debug
    for RunState<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>
{
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
        fmt.debug_struct("RunState")
            .field("range", &self.range)
            .field("code", &self.code)
            .finish()
    }
}

#[derive(Debug)]
pub enum StreamStatus {
    ProcessingHeader,
    ProcessingData {
        unpacked_data_processed: u64,
        unpacked_size: Option<u64>,
    },
    InvalidState,
    Finished,
}

/// Lzma decompressor that can process multiple chunks of data using the
/// `io::Write` interface.
pub struct Stream<W, const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize>
where
    W: Write,
{
    decoder: DecoderState<W, LzCircularBuffer<DICT_MEM_LIMIT>, PROBS_MEM_LIMIT>,
    /// Temporary buffer to hold data while the header is being read.
    tmp: Cursor<[u8; MAX_TMP_LEN]>,
    /// Whether the stream is initialized and ready to process data.
    /// An `Option` is used to avoid interior mutability when updating the
    /// state.
    state: Option<State<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>>,
    /// Options given when a stream is created.
    options: Options,
}

impl<W, const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize>
    Stream<W, DICT_MEM_LIMIT, PROBS_MEM_LIMIT>
where
    W: Write,
{
    /// Initialize the stream. This will consume the `output` which is the sink
    /// implementing `io::Write` that will receive decompressed bytes.
    pub fn new() -> Self {
        Self::new_with_options(&Options::default())
    }

    /// Initialize the stream with the given `options`. This will consume the
    /// `output` which is the sink implementing `io::Write` that will
    /// receive decompressed bytes.
    pub fn new_with_options(options: &Options) -> Self {
        Self {
            decoder: Default::default(),
            tmp: Cursor::new([0; MAX_TMP_LEN]),
            state: Some(State::Header),
            options: *options,
        }
    }

    pub fn reset(&mut self) {
        self.decoder.reset();
        self.tmp = Cursor::new([0; MAX_TMP_LEN]);
        self.state = Some(State::Header);
    }

    /// Consumes the stream and returns the output sink. This also makes sure
    /// we have properly reached the end of the stream.
    pub fn finish(&mut self, output: &mut W) -> crate::error::Result<()> {
        let finish_status = if let Some(state) = self.state.take() {
            match state {
                State::Header => {
                    if self.tmp.position() > 0 {
                        Err(error::stream::StreamError::FailedToReadLzmaHeader.into())
                    } else {
                        Ok(())
                    }
                }
                State::Data(mut state) => {
                    // Process one last time with empty input to force end of
                    // stream checks
                    let mut stream =
                        Cursor::new(&self.tmp.get_ref()[0..self.tmp.position() as usize]);
                    let mut range_decoder =
                        RangeDecoder::from_parts(&mut stream, state.range, state.code);
                    self.decoder
                        .process(output, &mut range_decoder)
                        .and(self.decoder.output.finish(output).map_err(|e| e.into()))
                        .and(Ok(()))
                }
            }
        } else {
            // this will occur if a call to `write()` fails
            Err(error::stream::StreamError::InvalidState.into())
        };
        self.reset();
        finish_status
    }

    /// Attempts to read the header and transition into a running state.
    ///
    /// This function will consume the state, returning the next state on both
    /// error and success.
    fn read_header<R: BufRead>(
        decoder: &mut DecoderState<W, LzCircularBuffer<DICT_MEM_LIMIT>, PROBS_MEM_LIMIT>,
        mut input: &mut R,
        options: &Options,
    ) -> crate::error::Result<State<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>> {
        match LzmaParams::read_header(&mut input, options) {
            Ok(params) => {
                // The RangeDecoder is only kept temporarily as we are processing
                // chunks of data.
                if let Ok(rangecoder) = RangeDecoder::new(&mut input) {
                    decoder.set_params(params)?;
                    Ok(State::Data(RunState {
                        range: rangecoder.range,
                        code: rangecoder.code,
                    }))
                } else {
                    // Failed to create a RangeDecoder because we need more data,
                    // try again later.
                    Ok(State::Header)
                }
            }
            // Failed to read_header() because we need more data, try again later.
            Err(error::Error::HeaderTooShort(_)) => Ok(State::Header),
            // Fatal error. Don't retry.
            Err(e) => Err(e),
        }
    }

    /// Process compressed data
    fn read_data<R: BufRead>(
        decoder: &mut DecoderState<W, LzCircularBuffer<DICT_MEM_LIMIT>, PROBS_MEM_LIMIT>,
        mut state: RunState<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>,
        output: &mut W,
        mut input: &mut R,
    ) -> crate::error::Result<RunState<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>> {
        // Construct our RangeDecoder from the previous range and code
        // values.
        let mut rangecoder = RangeDecoder::from_parts(&mut input, state.range, state.code);

        // Try to process all bytes of data.
        decoder.process_stream(output, &mut rangecoder)?;

        Ok(RunState {
            range: rangecoder.range,
            code: rangecoder.code,
        })
    }

    pub fn write(&mut self, output: &mut W, data: &[u8]) -> crate::error::Result<usize> {
        let mut input = Cursor::new(data);

        if let Some(state) = self.state.take() {
            let state = match state {
                // Read the header values and transition into a running state.
                State::Header => {
                    let res = if self.tmp.position() > 0 {
                        // attempt to fill the tmp buffer
                        let position = self.tmp.position();
                        let bytes_read =
                            input.read(&mut self.tmp.get_mut()[position as usize..])?;
                        let bytes_read = if bytes_read < u64::MAX as usize {
                            bytes_read as u64
                        } else {
                            return Err(io::Error::new(
                                io::ErrorKind::Other,
                                "Failed to convert integer to u64.",
                            )
                            .into());
                        };
                        self.tmp.set_position(position + bytes_read);

                        // attempt to read the header from our tmp buffer
                        let (position, res) = {
                            let mut tmp_input =
                                Cursor::new(&self.tmp.get_ref()[0..self.tmp.position() as usize]);
                            let res = Stream::read_header(
                                &mut self.decoder,
                                &mut tmp_input,
                                &self.options,
                            );
                            (tmp_input.position(), res)
                        };

                        // discard all bytes up to position if reading the header
                        // was successful
                        if let Ok(State::Data(_)) = &res {
                            let tmp = *self.tmp.get_ref();
                            let end = self.tmp.position();
                            let new_len = end - position;
                            (&mut self.tmp.get_mut()[0..new_len as usize])
                                .copy_from_slice(&tmp[position as usize..end as usize]);
                            self.tmp.set_position(new_len);
                        }
                        res
                    } else {
                        Stream::read_header(&mut self.decoder, &mut input, &self.options)
                    }?;

                    match res {
                        // occurs when not enough input bytes were provided to
                        // read the entire header
                        State::Header => {
                            if self.tmp.position() == 0 {
                                // reset the cursor because we may have partial reads
                                input.set_position(0);
                                let bytes_read = input.read(&mut self.tmp.get_mut()[..])?;
                                let bytes_read = if bytes_read < u64::MAX as usize {
                                    bytes_read as u64
                                } else {
                                    return Err(io::Error::new(
                                        io::ErrorKind::Other,
                                        "Failed to convert integer to u64.",
                                    )
                                    .into());
                                };
                                self.tmp.set_position(bytes_read);
                            }
                            State::Header
                        }

                        // occurs when the header was successfully read and we
                        // move on to the next state
                        State::Data(val) => State::Data(val),
                    }
                }

                // Process another chunk of data.
                State::Data(state) => {
                    let state = if self.tmp.position() > 0 {
                        let mut tmp_input =
                            Cursor::new(&self.tmp.get_ref()[0..self.tmp.position() as usize]);
                        let res =
                            Stream::read_data(&mut self.decoder, state, output, &mut tmp_input)?;
                        self.tmp.set_position(0);
                        res
                    } else {
                        state
                    };
                    State::Data(Stream::read_data(
                        &mut self.decoder,
                        state,
                        output,
                        &mut input,
                    )?)
                }
            };
            self.state.replace(state);
        }
        Ok(input.position() as usize)
    }

    pub fn write_all(&mut self, output: &mut W, mut buf: &[u8]) -> crate::error::Result<()> {
        while !buf.is_empty() {
            match self.write(output, buf) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write whole buffer",
                    )
                    .into());
                }
                Ok(n) => buf = &buf[n..],
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn get_stream_status(&self) -> StreamStatus {
        use State::*;
        use StreamStatus::*;
        match &self.state {
            Some(Header) => ProcessingHeader,
            Some(Data(_)) => {
                let unpacked_size = self
                    .decoder
                    .params
                    .clone()
                    .unwrap_or_else(|| panic!("DecoderState::params is not initialized"))
                    .unpacked_size;
                let unpacked_data_processed =
                    LzBuffer::<W>::len(&self.decoder.output) as u64 + self.tmp.position();
                if let Some(unpacked_size) = unpacked_size {
                    if unpacked_size == unpacked_data_processed {
                        return Finished;
                    }
                }
                ProcessingData {
                    unpacked_size,
                    unpacked_data_processed,
                }
            }
            None => InvalidState,
        }
    }
}

impl<W, const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize> Debug
    for Stream<W, DICT_MEM_LIMIT, PROBS_MEM_LIMIT>
where
    W: Write + Debug,
{
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
        fmt.debug_struct("Stream")
            .field("tmp", &self.tmp.position())
            .field("state", &self.state)
            .field("options", &self.options)
            .finish()
    }
}

#[cfg(all(test, feature = "std"))]
mod test {
    use super::*;

    /// Test an empty stream
    #[test]
    fn test_stream_noop() {
        let mut sink = Vec::new();
        let mut stream = Stream::<_, 4096, 8>::new();

        stream.finish(&mut sink).unwrap();
        assert!(sink.is_empty());
    }

    /// Test writing an empty slice
    #[test]
    fn test_stream_zero() {
        let mut sink = Vec::new();
        let mut stream = Stream::<_, 4096, 8>::new();

        stream.write_all(&mut sink, &[]).unwrap();
        stream.write_all(&mut sink, &[]).unwrap();

        stream.finish(&mut sink).unwrap();

        assert!(sink.is_empty());
    }

    /// Test a bad header value
    #[test]
    fn test_bad_header() {
        let input = [255u8; 32];

        let mut sink = Vec::new();
        let mut stream = Stream::<_, 4096, 8>::new();

        match stream.write_all(&mut sink, &input[..]).unwrap_err() {
            error::Error::LzmaError(error::lzma::LzmaError::InvalidHeader {
                invalid_properties: 255,
            }) => {}
            err => panic!("Unexpected error: {:#?}", err),
        }

        match stream.finish(&mut sink).unwrap_err() {
            error::Error::StreamError(error::stream::StreamError::InvalidState) => {}
            err => panic!("Unexpected error: {:#?}", err),
        }

        assert!(sink.is_empty());
    }

    /// Test processing only partial data
    #[test]
    fn test_stream_incomplete() {
        let input = b"\x5d\x00\x10\x00\x00\xff\xff\xff\xff\xff\xff\xff\xff\x00\x83\xff\
                      \xfb\xff\xff\xc0\x00\x00\x00";
        // Process until this index is reached.
        let mut end = 1u64;

        // Test when we fail to provide the minimum number of bytes required to
        // read the header. Header size is 13 bytes but we also read the first 5
        // bytes of data.
        while end < (MAX_HEADER_LEN + START_BYTES) as u64 {
            let mut sink = Vec::new();
            let mut stream = Stream::<_, 4096, 8>::new();
            stream.write_all(&mut sink, &input[..end as usize]).unwrap();
            assert_eq!(stream.tmp.position(), end);

            match stream.finish(&mut sink).unwrap_err() {
                error::Error::StreamError(error::stream::StreamError::FailedToReadLzmaHeader) => {}
                err => panic!("Unexpected error: {:#?}", err),
            }

            end += 1;
        }

        // Test when we fail to provide enough bytes to terminate the stream. A
        // properly terminated stream will have a code value of 0.
        while end < input.len() as u64 {
            let mut sink = Vec::new();
            let mut stream = Stream::<_, 4096, 8>::new();
            stream.write_all(&mut sink, &input[..end as usize]).unwrap();

            // Header bytes will be buffered until there are enough to read
            if end < (MAX_HEADER_LEN + START_BYTES) as u64 {
                assert_eq!(stream.tmp.position(), end);
            }

            match stream.finish(&mut sink).unwrap_err() {
                error::Error::IoError(io_error) => {
                    assert!(io_error.to_string().contains("failed to fill whole buffer"))
                }
                err => panic!("Unexpected error: {:#?}", err),
            }

            end += 1;
        }
    }

    /// Test processing all chunk sizes
    #[test]
    fn test_stream_chunked() {
        let small_input = include_bytes!("../../tests/files/small.txt");

        let mut reader = io::Cursor::new(&small_input[..]);
        let mut small_input_compressed = Vec::new();
        crate::lzma_compress(&mut reader, &mut small_input_compressed).unwrap();

        let input : Vec<(&[u8], &[u8])> = vec![
            (b"\x5d\x00\x10\x00\x00\xff\xff\xff\xff\xff\xff\xff\xff\x00\x83\xff\xfb\xff\xff\xc0\x00\x00\x00", b""),
            (&small_input_compressed[..], small_input)];
        for (input, expected) in input {
            for chunk in 1..input.len() {
                let mut consumed = 0;
                let mut sink = Vec::new();
                let mut stream = Stream::<_, 4096, 8>::new();
                while consumed < input.len() {
                    let end = std::cmp::min(consumed + chunk, input.len());
                    stream.write_all(&mut sink, &input[consumed..end]).unwrap();
                    consumed = end;
                }
                stream.finish(&mut sink).unwrap();
                assert_eq!(expected, &sink[..]);
            }
        }
    }

    #[test]
    fn test_stream_corrupted() {
        let mut sink = Vec::new();
        let mut stream = Stream::<_, 4096, 8>::new();
        let _ = stream
            .write_all(&mut sink, b"corrupted bytes here corrupted bytes here")
            .unwrap_err();

        match stream.finish(&mut sink).unwrap_err() {
            error::Error::StreamError(error::stream::StreamError::InvalidState) => {}
            err => panic!("Unexpected error: {:#?}", err),
        }
    }
}
