#![feature(test)]

extern crate test;

use std::io::Read;
use test::Bencher;

fn decompress_after_compress_bench(x: &[u8], b: &mut Bencher) {
    let mut compressed: Vec<u8> = Vec::new();
    lzma_rs::lzma_compress(&mut std::io::BufReader::new(x), &mut compressed).unwrap();

    b.iter(|| {
        let mut bf = std::io::BufReader::new(compressed.as_slice());
        let mut decomp: Vec<u8> = Vec::new();
        lzma_rs::lzma_decompress::<_, _, 4096, 8>(&mut bf, &mut decomp).unwrap();
        decomp
    });
}

fn decompress_bench<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize>(
    compressed: &[u8],
    b: &mut Bencher,
) {
    b.iter(|| {
        let mut bf = std::io::BufReader::new(compressed);
        let mut decomp: Vec<u8> = Vec::new();
        lzma_rs::lzma_decompress::<_, _, DICT_MEM_LIMIT, PROBS_MEM_LIMIT>(&mut bf, &mut decomp)
            .unwrap();
        decomp
    });
}

#[cfg(feature = "stream")]
fn decompress_stream_bench<const DICT_MEM_LIMIT: usize, const PROBS_MEM_LIMIT: usize>(
    compressed: &[u8],
    b: &mut Bencher,
) {
    use std::io::Write;
    b.iter(|| {
        let mut sink = Vec::new();
        let mut stream = lzma_rs::decompress::Stream::<DICT_MEM_LIMIT, PROBS_MEM_LIMIT>::new();
        stream.reset();
        stream.write_all(&mut sink, compressed).unwrap();
        stream.finish(&mut sink).unwrap()
    });
}

fn decompress_bench_file(compfile: &str, b: &mut Bencher) {
    let mut f = std::fs::File::open(compfile).unwrap();
    let mut compressed = Vec::new();
    f.read_to_end(&mut compressed).unwrap();
    decompress_bench::<4096, 8>(&compressed, b);
}

#[cfg(feature = "stream")]
fn decompress_stream_bench_file(compfile: &str, b: &mut Bencher) {
    let mut f = std::fs::File::open(compfile).unwrap();
    let mut compressed = Vec::new();
    f.read_to_end(&mut compressed).unwrap();
    decompress_stream_bench::<4096, 8>(&compressed, b);
}

#[bench]
fn decompress_after_compress_empty(b: &mut Bencher) {
    #[cfg(feature = "enable_logging")]
    let _ = env_logger::try_init();
    decompress_after_compress_bench(b"", b);
}

#[bench]
fn decompress_after_compress_hello(b: &mut Bencher) {
    #[cfg(feature = "enable_logging")]
    let _ = env_logger::try_init();
    decompress_after_compress_bench(b"Hello world", b);
}

#[bench]
fn decompress_after_compress_65536(b: &mut Bencher) {
    #[cfg(feature = "enable_logging")]
    let _ = env_logger::try_init();
    decompress_after_compress_bench(&[0; 0x10000], b);
}

#[bench]
fn decompress_big_file(b: &mut Bencher) {
    #[cfg(feature = "enable_logging")]
    let _ = env_logger::try_init();
    decompress_bench_file("tests/files/foo.txt.lzma", b);
}

#[cfg(feature = "stream")]
#[bench]
fn decompress_stream_big_file(b: &mut Bencher) {
    #[cfg(feature = "enable_logging")]
    let _ = env_logger::try_init();
    decompress_stream_bench_file("tests/files/foo.txt.lzma", b);
}

#[bench]
fn decompress_huge_dict(b: &mut Bencher) {
    #[cfg(feature = "enable_logging")]
    let _ = env_logger::try_init();
    let compressed: &[u8] = b"\x5d\x00\x40\x00\x00\xff\xff\xff\
                              \xff\xff\xff\xff\xff\x00\x24\x19\
                              \x49\x98\x6f\x10\x19\xc6\xd7\x31\
                              \xeb\x36\x50\xb2\x98\x48\xff\xfe\
                              \xa5\xb0\x00";
    decompress_bench::<16384, 8>(&compressed, b);
}
