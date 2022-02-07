# Fork specific information

This is a fork of `lzma-rs` project that provides no-std and no-alloc
implementation of LZMA decompressor. All abstractions use only stack memory
and upper-bound memory usage is limited via const generic. `Stream` drops
`core2::io::Write` implementation in favour of custom `write` routine in
order to avoid storing the output sink inside of it. Stream can be also reset
to inital state without move/consume semantics to avoid sudden stack usage
spikes. All of this was done in order to make the library more suitable to
work with embedded targets; in particular with RTIC resource management model.

Fork drops support for everything beside lzma decompression. Dummy encoder
is kept (only `std`) to maintain test suite.

# lzma-rs

[![Crate](https://img.shields.io/crates/v/lzma-rs.svg)](https://crates.io/crates/lzma-rs)
[![Documentation](https://docs.rs/lzma-rs/badge.svg)](https://docs.rs/lzma-rs)
[![Safety Dance](https://img.shields.io/badge/unsafe-forbidden-success.svg)](https://github.com/rust-secure-code/safety-dance/)
![Build Status](https://github.com/gendx/lzma-rs/workflows/Build%20and%20run%20tests/badge.svg)
[![Minimum rust 1.40](https://img.shields.io/badge/rust-1.40%2B-orange.svg)](https://github.com/rust-lang/rust/blob/master/RELEASES.md#version-1400-2019-12-19)

This project is a decoder for LZMA and its variants written in pure Rust, with focus on clarity.
It already supports LZMA, LZMA2 and a subset of the `.xz` file format.

## Usage

Decompress a `.xz` file.

```rust
let filename = "foo.xz";
let mut f = std::io::BufReader::new(std::fs::File::open(filename).unwrap());
// "decomp" can be anything that implements "std::io::Write"
let mut decomp: Vec<u8> = Vec::new();
lzma_rs::xz_decompress(&mut f, &mut decomp).unwrap();
// Decompressed content is now in "decomp"
```

## Encoder

For now, there is also a dumb encoder that only uses byte literals, with many hard-coded constants for code simplicity.
Better encoders are welcome!

## Contributing

Pull-requests are welcome, to improve the decoder, add better encoders, or more tests.
Ultimately, this project should also implement .xz and .7z files.

## License

MIT

