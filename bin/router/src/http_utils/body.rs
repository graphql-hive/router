use std::{cell::RefMut, io::Write};

use http::header::CONTENT_LENGTH;
use ntex::{
    http::error::PayloadError,
    util::Extensions,
    web::{self, DefaultError, HttpRequest, WebRequest},
};
use ntex_bytes::{Bytes, BytesMut};
use ntex_http::HeaderMap;
use strum::IntoStaticStr;
use tokio_stream::StreamExt;

use crate::config::traffic_shaping::CompressionAlgorithm;

/// Stores the request body size in bytes.
///
/// The value comes from either:
/// - the `Content-Length` header
/// - the streamed payload, measured from bytes read.
///
/// For streamed payloads, the recorded size is the number of bytes read up to
/// the configured maximum.
///
/// Using `RequestBodySize` to store the size of the request body,
/// helps to reduce complexity in code, as otherwise,
/// we would have to return the size next within Err and Ok of `read_body_stream`.
#[derive(Debug, Clone, Copy)]
pub struct RequestBodySize(pub u64);

#[derive(Debug, thiserror::Error, IntoStaticStr)]
pub enum ReadBodyStreamError {
    #[error("Failed to read request body: {0}")]
    #[strum(serialize = "PAYLOAD_READ_ERROR")]
    // Thrown while reading the body stream with `try_next()`
    PayloadReadError(#[from] PayloadError),

    #[error("Content-Length header has invalid value")]
    #[strum(serialize = "INVALID_HEADER")]
    InvalidContentLengthHeader,

    #[error("Content-Length exceeds the maximum allowed size: {0}")]
    #[strum(serialize = "PAYLOAD_TOO_LARGE_CONTENT_LENGTH")]
    PayloadTooLargeContentLength(usize),

    #[error("Request body exceeds the maximum allowed size while reading the stream")]
    #[strum(serialize = "PAYLOAD_TOO_LARGE_BODY_STREAM")]
    PayloadTooLargeBodyStream,

    #[error("Failed to decompress request body: {0}")]
    #[strum(serialize = "DECOMPRESSION_FAILED")]
    DecompressionFailed(String),
}

impl ReadBodyStreamError {
    pub fn status_code(&self) -> http::StatusCode {
        match self {
            Self::PayloadReadError(_) => http::StatusCode::UNPROCESSABLE_ENTITY,
            Self::InvalidContentLengthHeader => http::StatusCode::BAD_REQUEST,
            Self::PayloadTooLargeContentLength(_) | Self::PayloadTooLargeBodyStream => {
                http::StatusCode::PAYLOAD_TOO_LARGE
            }
            Self::DecompressionFailed(_) => http::StatusCode::BAD_REQUEST,
        }
    }

    pub fn error_code(&self) -> &'static str {
        self.into()
    }
}

#[inline]
fn write_request_body_size<R: RequestLike>(req: &R, size: u64) {
    req.extensions_mut().insert(RequestBodySize(size));
}

#[inline]
pub fn read_request_body_size(req: &HttpRequest) -> Option<u64> {
    req.extensions().get::<RequestBodySize>().map(|size| size.0)
}

/// Limit for draining a rejected request body. Keeps the connection
/// reusable for typical over-limit requests without reading the whole thing
const MAX_DRAIN_BYTES: usize = 64 * 1024;

pub async fn drain_body_stream(body_stream: &mut web::types::Payload) {
    let mut drained: usize = 0;

    while drained < MAX_DRAIN_BYTES {
        match body_stream.try_next().await {
            Ok(Some(chunk)) => {
                if chunk.is_empty() {
                    break;
                }

                drained = drained.saturating_add(chunk.len());
            }
            _ => break,
        }
    }
}

#[inline]
pub async fn read_body_stream<R: RequestLike>(
    req: &R,
    mut body_stream: web::types::Payload,
    max_size: usize,
    content_encoding: Option<CompressionAlgorithm>,
) -> Result<Bytes, ReadBodyStreamError> {
    let content_length: Option<usize> = {
        let content_length_header = req.headers().get(CONTENT_LENGTH);
        if let Some(content_length_header) = content_length_header {
            let content_length_str = content_length_header
                .to_str()
                .map_err(|_| ReadBodyStreamError::InvalidContentLengthHeader)?;
            let content_length: usize = content_length_str
                .parse()
                .map_err(|_| ReadBodyStreamError::InvalidContentLengthHeader)?;
            if content_length > max_size {
                write_request_body_size(req, content_length as u64);

                // Drain just small amount of the request body before rejecting,
                // so the server can close the connection cleanly.
                //
                // Returning without consuming the body makes ntex reset the socket,
                // and the client might read that as a socket error instead of the router's 413
                // response.
                //
                // Note: `drain_body_stream` reads only a small amount of the body,
                // so the client will not be blocked while draining.
                drain_body_stream(&mut body_stream).await;
                return Err(ReadBodyStreamError::PayloadTooLargeContentLength(max_size));
            }
            Some(content_length)
        } else {
            None
        }
    };

    let Some(encoding) = content_encoding else {
        let mut body = if let Some(content_length) = content_length {
            BytesMut::with_capacity(content_length)
        } else {
            BytesMut::new()
        };

        while let Some(chunk) = body_stream.try_next().await? {
            // limit max size of in-memory payload
            if chunk.len() > max_size.saturating_sub(body.len()) {
                write_request_body_size(req, (body.len() + chunk.len()) as u64);
                return Err(ReadBodyStreamError::PayloadTooLargeBodyStream);
            }
            body.extend_from_slice(&chunk);
        }

        write_request_body_size(req, body.len() as u64);
        return Ok(body.freeze());
    };

    // Decompress incrementally as wire chunks arrive. `RequestBodyDecoder` writes into a
    // `SizeLimitedSink` that rejects growth past `max_size` as it happens (inside the
    // decoders' own internal write calls), rather than only checking the total after each
    // whole wire chunk has already been fully inflated - see `SizeLimitedSink` below for why
    // that distinction matters for a highly compressible payload.
    let mut decoder = RequestBodyDecoder::new(encoding, max_size)
        .map_err(|err| ReadBodyStreamError::DecompressionFailed(err.to_string()))?;

    while let Some(chunk) = body_stream.try_next().await? {
        if let Err(err) = decoder.write_all(&chunk) {
            if decoder.size_limit_exceeded() {
                write_request_body_size(req, decoder.len() as u64);
                drain_body_stream(&mut body_stream).await;
                return Err(ReadBodyStreamError::PayloadTooLargeBodyStream);
            }
            return Err(ReadBodyStreamError::DecompressionFailed(err.to_string()));
        }
    }

    let body = decoder.finish().map_err(|err| {
        if err.get_ref().is_some_and(|e| e.is::<SizeLimitExceeded>()) {
            ReadBodyStreamError::PayloadTooLargeBodyStream
        } else {
            ReadBodyStreamError::DecompressionFailed(err.to_string())
        }
    })?;

    write_request_body_size(req, body.len() as u64);

    Ok(body)
}

#[derive(Debug)]
struct SizeLimitExceeded;

impl std::fmt::Display for SizeLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "decompressed output exceeds the configured maximum request body size"
        )
    }
}

impl std::error::Error for SizeLimitExceeded {}

/// A `Write` sink that rejects growth past `max_size` as soon as it would happen, instead of
/// only being checked after an entire wire chunk has already been fully inflated into memory
struct SizeLimitedSink {
    buf: Vec<u8>,
    max_size: usize,
    exceeded: bool,
}

impl SizeLimitedSink {
    fn new(max_size: usize) -> Self {
        Self {
            buf: Vec::new(),
            max_size,
            exceeded: false,
        }
    }

    fn len(&self) -> usize {
        self.buf.len()
    }

    fn size_limit_exceeded(&self) -> bool {
        self.exceeded
    }
}

impl Write for SizeLimitedSink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.buf.len().saturating_add(data.len()) > self.max_size {
            self.exceeded = true;
            return Err(std::io::Error::other(SizeLimitExceeded));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

enum RequestBodyDecoder {
    Gzip(flate2::write::GzDecoder<SizeLimitedSink>),
    Deflate(flate2::write::ZlibDecoder<SizeLimitedSink>),
    Br(Box<brotli::DecompressorWriter<SizeLimitedSink>>),
    Zstd(Box<zstd::stream::write::Decoder<'static, SizeLimitedSink>>),
}

impl RequestBodyDecoder {
    fn new(algorithm: CompressionAlgorithm, max_size: usize) -> std::io::Result<Self> {
        Ok(match algorithm {
            CompressionAlgorithm::Gzip => Self::Gzip(flate2::write::GzDecoder::new(
                SizeLimitedSink::new(max_size),
            )),
            CompressionAlgorithm::Deflate => Self::Deflate(flate2::write::ZlibDecoder::new(
                SizeLimitedSink::new(max_size),
            )),
            CompressionAlgorithm::Br => Self::Br(Box::new(brotli::DecompressorWriter::new(
                SizeLimitedSink::new(max_size),
                4096,
            ))),
            CompressionAlgorithm::Zstd => Self::Zstd(Box::new(zstd::stream::write::Decoder::new(
                SizeLimitedSink::new(max_size),
            )?)),
        })
    }

    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            Self::Gzip(d) => d.write_all(data),
            Self::Deflate(d) => d.write_all(data),
            Self::Br(d) => d.write_all(data),
            Self::Zstd(d) => d.write_all(data),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Gzip(d) => d.get_ref().len(),
            Self::Deflate(d) => d.get_ref().len(),
            Self::Br(d) => d.get_ref().len(),
            Self::Zstd(d) => d.get_ref().len(),
        }
    }

    fn size_limit_exceeded(&self) -> bool {
        match self {
            Self::Gzip(d) => d.get_ref().size_limit_exceeded(),
            Self::Deflate(d) => d.get_ref().size_limit_exceeded(),
            Self::Br(d) => d.get_ref().size_limit_exceeded(),
            Self::Zstd(d) => d.get_ref().size_limit_exceeded(),
        }
    }

    fn finish(self) -> std::io::Result<Bytes> {
        let bytes = match self {
            Self::Gzip(d) => d.finish()?.buf,
            Self::Deflate(d) => d.finish()?.buf,
            Self::Br(mut d) => {
                d.close()
                    .map_err(|_| std::io::Error::other("brotli stream did not close cleanly"))?;
                d.into_inner()
                    .map_err(|_| std::io::Error::other("brotli stream did not finish cleanly"))?
                    .buf
            }
            Self::Zstd(mut d) => {
                d.flush()?;
                d.into_inner().buf
            }
        };
        Ok(Bytes::from(bytes))
    }
}

pub trait RequestLike {
    fn headers(&self) -> &HeaderMap;
    fn extensions_mut(&self) -> RefMut<'_, Extensions>;
}

impl RequestLike for HttpRequest {
    fn headers(&self) -> &HeaderMap {
        self.headers()
    }

    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.extensions_mut()
    }
}

impl RequestLike for WebRequest<DefaultError> {
    fn headers(&self) -> &HeaderMap {
        self.headers()
    }

    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.extensions_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gzip(data: &[u8]) -> Vec<u8> {
        use flate2::{write::GzEncoder, Compression};
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn deflate(data: &[u8]) -> Vec<u8> {
        use flate2::{write::ZlibEncoder, Compression};
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn brotli_compress(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut writer = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
        writer.write_all(data).unwrap();
        drop(writer);
        out
    }

    fn zstd_compress(data: &[u8]) -> Vec<u8> {
        zstd::stream::encode_all(data, 3).unwrap()
    }

    // Directly proves the fix for a decompression bomb arriving as a single wire chunk: the
    // sink must reject growth as it happens *during* the decoder's internal write calls, so
    // the backing buffer never grows past `max_size` - not just that the caller eventually
    // gets an error back (which alone wouldn't rule out having fully inflated the payload
    // into memory first).
    #[test]
    fn size_limited_sink_bounds_backing_buffer_for_all_algorithms() {
        let max_size = 1024; // 1KiB
        let inflated = vec![b'a'; 5_000_000]; // 5MB, highly compressible

        let cases: [(CompressionAlgorithm, Vec<u8>); 4] = [
            (CompressionAlgorithm::Gzip, gzip(&inflated)),
            (CompressionAlgorithm::Deflate, deflate(&inflated)),
            (CompressionAlgorithm::Br, brotli_compress(&inflated)),
            (CompressionAlgorithm::Zstd, zstd_compress(&inflated)),
        ];

        for (algorithm, compressed) in cases {
            let mut decoder = RequestBodyDecoder::new(algorithm, max_size)
                .unwrap_or_else(|e| panic!("[{algorithm:?}] failed to construct decoder: {e}"));

            // the whole compressed payload arrives as a single `write_all` call, mirroring a
            // single wire chunk delivering an extreme compression ratio in one shot.
            let result = decoder.write_all(&compressed);

            assert!(
                result.is_err(),
                "[{algorithm:?}] expected write_all to fail once decompressed output exceeds max_size"
            );
            assert!(
                decoder.size_limit_exceeded(),
                "[{algorithm:?}] expected the sink to flag the size limit as exceeded"
            );
            assert!(
                decoder.len() <= max_size,
                "[{algorithm:?}] backing buffer grew to {} bytes, past the {max_size}-byte limit",
                decoder.len()
            );
        }
    }

    #[test]
    fn size_limited_sink_allows_output_at_or_under_the_limit() {
        let max_size = 1024;
        // small enough that its decompressed output stays under max_size
        let small = vec![b'a'; 100];

        for (algorithm, compressed) in [
            (CompressionAlgorithm::Gzip, gzip(&small)),
            (CompressionAlgorithm::Deflate, deflate(&small)),
            (CompressionAlgorithm::Br, brotli_compress(&small)),
            (CompressionAlgorithm::Zstd, zstd_compress(&small)),
        ] {
            let mut decoder = RequestBodyDecoder::new(algorithm, max_size)
                .unwrap_or_else(|e| panic!("[{algorithm:?}] failed to construct decoder: {e}"));

            decoder
                .write_all(&compressed)
                .unwrap_or_else(|e| panic!("[{algorithm:?}] unexpected write_all failure: {e}"));

            let body = decoder
                .finish()
                .unwrap_or_else(|e| panic!("[{algorithm:?}] unexpected finish failure: {e}"));

            assert_eq!(
                body.as_ref(),
                small.as_slice(),
                "[{algorithm:?}] roundtrip mismatch"
            );
        }
    }
}
