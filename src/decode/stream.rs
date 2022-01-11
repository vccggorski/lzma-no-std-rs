use crate::decode::lzbuffer::{LzBuffer, LzCircularBuffer};
use crate::decode::lzma::{DecoderState, LzmaParams};
use crate::decode::rangecoder::RangeDecoder;
use crate::decompress::Options;
use crate::error;
use crate::io::{self, BufRead, Cursor, Read, Write};
use crate::option::GuaranteedOption as Option;
use crate::option::GuaranteedOption::*;
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
#[repr(C)]
#[derive(Debug)]
enum State<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize> {
    Uninitialized,
    InvalidState,
    /// Stream is initialized but header values have not yet been read.
    Header,
    /// Header values have been read and the stream is ready to process more
    /// data.
    Data(RunState<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>),
}

impl<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize>
    State<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>
{
    fn take(&mut self) -> Self {
        core::mem::replace(self, Self::InvalidState)
    }
    fn replace(&mut self, value: Self) -> Self {
        core::mem::replace(self, value)
    }
}

/// Structures needed while decoding data.
#[derive(Debug)]
struct RunState<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize> {
    range: u32,
    code: u32,
}

/// Enum describing current state of a stream
#[derive(PartialEq, Debug)]
pub enum StreamStatus {
    /// Stream has not been initialized; call [`Stream::reset`]
    Uninitialized,
    /// LZMA header is currently being processed
    ProcessingHeader,
    /// LZMA data stream is currently being processed
    ProcessingData {
        /// Data that has been already decompressed in bytes.
        /// Note: Call to [`Stream::finish`] is required; write to the output
        /// might not have happened yet
        unpacked_data_processed: u64,
        /// Expected unpacked size (behaviour of decoder depends on
        /// [`crate::decode::options::Options::unpacked_size`] setting)
        unpacked_size: core::option::Option<u64>,
    },
    /// Stream entered undefined state. Happens if one calls `Stream::finish`
    /// after faulty `Stream::write` call
    InvalidState,
    /// End-Of-Stream marker has been reached
    EosReached,
}

/// Lzma decompressor that can process multiple chunks of data using the
/// `io::Write` interface.
///
/// - `DICT_MEM_LIMIT` must be equal or larger than dictionary size of
///   compressed data streams that will be processed
/// - `PROBS_MEM_LIMIT` must be equal or larger than (1 << LC + PB)
///   parametrization of compressed data streams that will be processed
pub struct Stream<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize> {
    decoder: DecoderState<LzCircularBuffer<DICT_MEM_LIMIT>, PROBS_MEM_LIMIT>,
    /// Temporary buffer to hold data while the header is being read.
    tmp: Cursor<[u8; MAX_TMP_LEN]>,
    /// Whether the stream is initialized and ready to process data.
    /// An `Option` is used to avoid interior mutability when updating the
    /// state.
    state: State<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>,
    /// Options given when a stream is created.
    options: Options,
}

impl<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize>
    Stream<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>
{
    /// Initialize the stream. This will consume the `output` which is the sink
    /// implementing `io::Write` that will receive decompressed bytes.
    pub const fn new() -> Self {
        Self::new_with_options(&Options::default())
    }

    /// Initialize the stream with the given `options`. This will consume the
    /// `output` which is the sink implementing `io::Write` that will
    /// receive decompressed bytes.
    pub const fn new_with_options(options: &Options) -> Self {
        Self {
            decoder: DecoderState::new(),
            tmp: Cursor::new([0; MAX_TMP_LEN]),
            state: State::Uninitialized,
            options: *options,
        }
    }

    /// Reset the state of the stream. All internal buffers and fields are
    /// cleared and set to initial values.
    pub fn reset(&mut self) {
        self.decoder.reset();
        self.tmp = Cursor::new([0; MAX_TMP_LEN]);
        self.state = State::Header;
    }

    /// Consumes the stream and returns the output sink. This also makes sure
    /// we have properly reached the end of the stream.
    pub fn finish(&mut self, output: &mut dyn Write) -> crate::error::Result<()> {
        let finish_status = match self.state.take() {
            State::Header => {
                if self.tmp.position() > 0 {
                    Err(error::stream::StreamError::FailedToReadLzmaHeader.into())
                } else {
                    Ok(())
                }
            }
            State::Data(state) => {
                // Process one last time with empty input to force end of
                // stream checks
                let mut stream = Cursor::new(&self.tmp.get_ref()[0..self.tmp.position() as usize]);
                let mut range_decoder =
                    RangeDecoder::from_parts(&mut stream, state.range, state.code);
                self.decoder
                    .process(output, &mut range_decoder)
                    .and(self.decoder.output.finish(output).map_err(|e| e.into()))
                    .and(Ok(()))
            }
            State::InvalidState => Err(error::stream::StreamError::InvalidState.into()),
            State::Uninitialized => panic!("Stream is uninitialized; call `Stream::reset` first"),
        };
        self.reset();
        finish_status
    }

    /// Attempts to read the header and transition into a running state.
    ///
    /// This function will consume the state, returning the next state on both
    /// error and success.
    fn read_header<R: BufRead>(
        decoder: &mut DecoderState<LzCircularBuffer<DICT_MEM_LIMIT>, PROBS_MEM_LIMIT>,
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
        decoder: &mut DecoderState<LzCircularBuffer<DICT_MEM_LIMIT>, PROBS_MEM_LIMIT>,
        state: RunState<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>,
        output: &mut dyn Write,
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

    /// Write slice of compressed `data` into the stream. Decompressed data will
    /// be written to the `output` sink.
    ///
    /// This function reads between 0 and `data.len()` of bytes. To read all the
    /// data from `data` slice, use [`Stream::write_all`] function.
    pub fn write(&mut self, output: &mut dyn Write, data: &[u8]) -> crate::error::Result<usize> {
        if let StreamStatus::Uninitialized = self.get_stream_status() {
            panic!("Stream is uninitialized; call `Stream::reset` first");
        }
        let mut input = Cursor::new(data);

        let state = match self.state.take() {
            // Read the header values and transition into a running state.
            State::Header => {
                let res = if self.tmp.position() > 0 {
                    // attempt to fill the tmp buffer
                    let position = self.tmp.position();
                    let bytes_read = input.read(&mut self.tmp.get_mut()[position as usize..])?;
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
                        let res =
                            Stream::read_header(&mut self.decoder, &mut tmp_input, &self.options);
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
                    State::InvalidState => {
                        return Err(error::stream::StreamError::InvalidState.into())
                    }
                    State::Uninitialized => {
                        panic!("Stream is uninitialized; call `Stream::reset` first")
                    }
                }
            }

            // Process another chunk of data.
            State::Data(state) => {
                let state = if self.tmp.position() > 0 {
                    let mut tmp_input =
                        Cursor::new(&self.tmp.get_ref()[0..self.tmp.position() as usize]);
                    let res = Stream::read_data(&mut self.decoder, state, output, &mut tmp_input)?;
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
            State::InvalidState => return Err(error::stream::StreamError::InvalidState.into()),
            State::Uninitialized => panic!("Stream is uninitialized; call `Stream::reset` first"),
        };
        self.state.replace(state);

        Ok(input.position() as usize)
    }

    ///
    pub fn write_all(
        &mut self,
        output: &mut dyn Write,
        mut buf: &[u8],
    ) -> crate::error::Result<()> {
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

    /// Retrieve the stream state.
    ///
    /// If [`StreamStatus::EosReached`] is returned, [`Stream::finish`] call is
    /// guaranteed not to fail.
    pub fn get_stream_status(&self) -> StreamStatus {
        use crate::decode::lzma::ProcessingStatus;
        use State::*;
        use StreamStatus::*;
        match &self.state {
            Header => ProcessingHeader,
            Data(_) => {
                let params = match &self.decoder.params {
                    Some(v) => v.clone(),
                    None => panic!(
                "DecoderState::params is not initialized; call `DecoderState::set_params` first"
            ),
                };
                let unpacked_size = params.unpacked_size;
                // Temporary buffer in `Stream` must be checked; without `Stream::finish` call,
                // not all bytes might have pushed into the decoder
                // TODO: What if
                // 1. unpacked-size field in the header is set
                // 2. EOS exists in the end of a stream
                // 3. Decoder reads all the data bytes but not the marker?
                // Status will then indicate that unpacked_size == unpacked_data_processed but
                // Eos is not reached yet. Should one call `finish` then?
                // Should tmp position be added to unpacked_data_processed?
                let unpacked_data_processed =
                    self.decoder.output.len() as u64 + self.tmp.position();
                // TODO: Add tests stressing this; especially considering different decoding
                // options in `decode::Options::UnpackedSize` If unpacked_size

                // If EOS marker is found, return proper status
                // Note: reaching unpacked_size == unpacked_data_processed does not mean that
                // stream has been terminated
                match self.decoder.get_processing_status() {
                    ProcessingStatus::Uninitialized => StreamStatus::Uninitialized,
                    ProcessingStatus::Continue => ProcessingData {
                        unpacked_size: unpacked_size.into(),
                        unpacked_data_processed,
                    },
                    ProcessingStatus::Finished => EosReached,
                }
            }
            State::InvalidState => StreamStatus::InvalidState,
            State::Uninitialized => StreamStatus::Uninitialized,
        }
    }
}

impl<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize> Debug
    for Stream<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>
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
    // TODO: Write a test that checks if resetting is always equal to newly
    // construted object
    use super::*;

    /// Test an empty stream
    #[test]
    fn test_stream_noop() {
        let mut sink = Vec::new();
        let mut stream = Stream::<4096, 8>::new();
        stream.reset();

        stream.finish(&mut sink).unwrap();
        assert!(sink.is_empty());
    }

    /// Test writing an empty slice
    #[test]
    fn test_stream_zero() {
        let mut sink = Vec::new();
        let mut stream = Stream::<4096, 8>::new();
        stream.reset();

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
        let mut stream = Stream::<4096, 8>::new();
        stream.reset();

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

    #[test]
    fn test_stream_oom_handling() {
        let input = b"\x5d\x00\x10\x00\x00\xff\xff\xff\xff\xff\xff\xff\xff\x00\x83\xff\
                      \xfb\xff\xff\xc0\x00\x00\x00";
        let mut sink = Vec::new();
        let mut stream = Stream::<0, 8>::new();
        stream.reset();
        match stream.write_all(&mut sink, &input[..]).unwrap_err() {
            error::Error::DictionaryBufferTooSmall {
                needed: 4096,
                available: 0,
            } => {}
            err => panic!("Unexpected error: {:#?}", err),
        }
        match stream.finish(&mut sink).unwrap_err() {
            error::Error::StreamError(error::stream::StreamError::InvalidState) => {}
            err => panic!("Unexpected error: {:#?}", err),
        }
        let mut sink = Vec::new();
        let mut stream = Stream::<4096, 0>::new();
        stream.reset();
        match stream.write_all(&mut sink, &input[..]).unwrap_err() {
            error::Error::ProbabilitiesBufferTooSmall {
                needed: 8,
                available: 0,
            } => {}
            err => panic!("Unexpected error: {:#?}", err),
        }
        match stream.finish(&mut sink).unwrap_err() {
            error::Error::StreamError(error::stream::StreamError::InvalidState) => {}
            err => panic!("Unexpected error: {:#?}", err),
        }
    }

    /// Test if `Stream` behaviour stays the same as long as capacities are sane
    #[test]
    fn test_stream_different_capacities() {
        use StreamStatus::*;
        let input = include_bytes!("../../tests/files/foo.txt.lzma");
        let expected = include_bytes!("../../tests/files/foo.txt");
        let mut sink = Vec::new();
        let mut stream = Stream::<4096, 8>::new();
        stream.reset();
        stream.write_all(&mut sink, &input[..]).unwrap();
        assert_eq!(stream.get_stream_status(), EosReached);
        stream.finish(&mut sink).unwrap();
        assert_eq!(expected, &sink[..]);
        let mut sink = Vec::new();
        let mut stream = Stream::<8000, 8>::new();
        stream.reset();
        stream.write_all(&mut sink, &input[..]).unwrap();
        assert_eq!(stream.get_stream_status(), EosReached);
        stream.finish(&mut sink).unwrap();
        assert_eq!(expected, &sink[..]);
        let mut sink = Vec::new();
        let mut stream = Stream::<66666, 30>::new();
        stream.reset();
        stream.write_all(&mut sink, &input[..]).unwrap();
        assert_eq!(stream.get_stream_status(), EosReached);
        stream.finish(&mut sink).unwrap();
        assert_eq!(expected, &sink[..]);
    }

    /// Test resetting capability of `Stream`
    #[test]
    fn test_stream_resetting() {
        let input = include_bytes!("../../tests/files/foo.txt.lzma");
        let expected = include_bytes!("../../tests/files/foo.txt");
        let mut sink = Vec::new();
        let mut stream = Stream::<4096, 8>::new();
        stream.reset();
        stream.write_all(&mut sink, &input[..]).unwrap();
        stream.finish(&mut sink).unwrap();
        assert_eq!(expected, &sink[..]);
        sink.truncate(0);
        stream.write_all(&mut sink, &input[..]).unwrap();
        stream.finish(&mut sink).unwrap();
        assert_eq!(expected, &sink[..]);
        sink.truncate(0);
        let (first_half, second_half) = input.split_at(input.len() / 2);
        stream.write_all(&mut sink, first_half).unwrap();
        stream.write_all(&mut sink, second_half).unwrap();
        stream.finish(&mut sink).unwrap();
        assert_eq!(expected, &sink[..]);
    }

    /// Test processing only partial data
    #[test]
    fn test_stream_incomplete() {
        use StreamStatus::*;
        let input = b"\x5d\x00\x10\x00\x00\xff\xff\xff\xff\xff\xff\xff\xff\x00\x83\xff\
                      \xfb\xff\xff\xc0\x00\x00\x00";
        // Process until this index is reached.
        let mut end = 1u64;

        // Test when we fail to provide the minimum number of bytes required to
        // read the header. Header size is 13 bytes but we also read the first 5
        // bytes of data.
        while end < (MAX_HEADER_LEN + START_BYTES) as u64 {
            let mut sink = Vec::new();
            let mut stream = Stream::<4096, 8>::new();
            stream.reset();
            stream.write_all(&mut sink, &input[..end as usize]).unwrap();
            assert_eq!(stream.tmp.position(), end);
            assert_eq!(stream.get_stream_status(), ProcessingHeader);

            match stream.finish(&mut sink).unwrap_err() {
                error::Error::StreamError(error::stream::StreamError::FailedToReadLzmaHeader) => {}
                err => panic!("Unexpected error: {:#?}", err),
            }
            // After `Stream::finish` call, stream state is reset
            assert_eq!(stream.get_stream_status(), ProcessingHeader);

            end += 1;
        }

        // Test when we fail to provide enough bytes to terminate the stream. A
        // properly terminated stream will have a code value of 0.
        while end < input.len() as u64 {
            let mut sink = Vec::new();
            let mut stream = Stream::<4096, 8>::new();
            stream.reset();
            stream.write_all(&mut sink, &input[..end as usize]).unwrap();
            match stream.get_stream_status() {
                ProcessingData {
                    unpacked_size: core::option::Option::<_>::None,
                    ..
                } => {}
                status => panic!("Unexpected status: {:#?}", status),
            }

            match stream.finish(&mut sink).unwrap_err() {
                error::Error::IoError(io_error) => {
                    assert!(io_error.to_string().contains("failed to fill whole buffer"))
                }
                err => panic!("Unexpected error: {:#?}", err),
            }
            // After `Stream::finish` call, stream state is reset
            assert_eq!(stream.get_stream_status(), ProcessingHeader);

            end += 1;
        }

        let mut sink = Vec::new();
        let mut stream = Stream::<4096, 8>::new();
        stream.reset();
        stream.write_all(&mut sink, &input[..end as usize]).unwrap();
        assert_eq!(stream.get_stream_status(), EosReached);

        stream.finish(&mut sink).unwrap();
        // After `Stream::finish` call, stream state is reset
        assert_eq!(stream.get_stream_status(), ProcessingHeader);
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
                let mut stream = Stream::<4096, 8>::new();
                stream.reset();
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
        let mut stream = Stream::<4096, 8>::new();
        stream.reset();
        stream.reset();
        let _ = stream
            .write_all(&mut sink, b"corrupted bytes here corrupted bytes here")
            .unwrap_err();

        match stream.finish(&mut sink).unwrap_err() {
            error::Error::StreamError(error::stream::StreamError::InvalidState) => {}
            err => panic!("Unexpected error: {:#?}", err),
        }
    }
}
