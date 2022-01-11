# lzma-no-std-rs

This is a fork of `lzma-rs` project that provides no-std and no-alloc
implementation of LZMA decompressor. All abstractions use only stack memory
and upper-bound memory usage is limited via const generics. `Stream` drops
`core2::io::Write` implementation in favour of custom `write` routine in
order to avoid storing the output sink inside of it. Stream can be also reset
to inital state without move/consume semantics to avoid sudden stack usage
spikes. All of this was done in order to make the library more suitable to
work with embedded targets; in particular with RTIC resource management model.

If `std` feature is enabled, `output` is expected to implement
`std::io::Write`. Otherwise, `core2::io::Write`.

Fork drops support for everything beside lzma decompression. Dummy encoder
is kept (only `std`) to maintain test suite.

## License

MIT

