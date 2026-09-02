use std::io::{Read, Write};

use crate::config::traffic_shaping::{
    BrotliCompressionConfig, CompressionAlgorithm, CompressionAlgorithmConfig,
    ZstdCompressionConfig,
};

pub fn compress(data: &[u8], algorithm: CompressionAlgorithmConfig) -> Option<Vec<u8>> {
    match algorithm {
        CompressionAlgorithmConfig::Gzip => gzip_compress(data),
        CompressionAlgorithmConfig::Deflate => deflate_compress(data),
        CompressionAlgorithmConfig::Br(config) => brotli_compress(data, config),
        CompressionAlgorithmConfig::Zstd(config) => zstd_compress(data, config),
    }
}

pub fn decompress(data: &[u8], algorithm: CompressionAlgorithm) -> Option<Vec<u8>> {
    match algorithm {
        CompressionAlgorithm::Gzip => gzip_decompress(data),
        CompressionAlgorithm::Deflate => deflate_decompress(data),
        CompressionAlgorithm::Br => brotli_decompress(data),
        CompressionAlgorithm::Zstd => zstd::stream::decode_all(data).ok(),
    }
}

fn gzip_compress(data: &[u8]) -> Option<Vec<u8>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).ok()?;
    encoder.finish().ok()
}

fn gzip_decompress(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

fn deflate_compress(data: &[u8]) -> Option<Vec<u8>> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).ok()?;
    encoder.finish().ok()
}

fn deflate_decompress(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

fn brotli_compress(data: &[u8], config: BrotliCompressionConfig) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut writer = brotli::CompressorWriter::new(&mut out, 4096, config.quality as u32, 22);
    writer.write_all(data).ok()?;
    drop(writer);
    Some(out)
}

fn brotli_decompress(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    brotli::Decompressor::new(data, 4096)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

fn zstd_compress(data: &[u8], config: ZstdCompressionConfig) -> Option<Vec<u8>> {
    zstd::stream::encode_all(data, config.level).ok()
}
