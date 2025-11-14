use async_compression::futures::bufread::{
    BrotliDecoder, DeflateDecoder, GzipDecoder, ZstdDecoder,
};
use bytes::Bytes;
use futures::{future::BoxFuture, AsyncReadExt};

use crate::executors::error::SubgraphExecutorError;

pub enum CompressionType {
    Gzip,
    Deflate,
    Brotli,
    Zstd,
    Multiple(Vec<CompressionType>),
    Identity,
}

impl CompressionType {
    pub fn accept_encoding() -> &'static str {
        "gzip, deflate, br, zstd"
    }
    pub fn header_value(&self) -> String {
        match self {
            CompressionType::Gzip => "gzip".to_string(),
            CompressionType::Deflate => "deflate".to_string(),
            CompressionType::Brotli => "br".to_string(),
            CompressionType::Zstd => "zstd".to_string(),
            CompressionType::Identity => "identity".to_string(),
            CompressionType::Multiple(types) => types
                .iter()
                .map(|t| t.header_value().to_string())
                .collect::<Vec<String>>()
                .join(", ")
                .to_string(),
        }
    }
    pub fn from_encoding_header(encoding: &str) -> Result<CompressionType, SubgraphExecutorError> {
        let encodings: Vec<&str> = encoding.split(',').map(|s| s.trim()).collect();
        if encodings.len() > 1 {
            let types = encodings
                .iter()
                .map(|&e| CompressionType::from_encoding_header(e))
                .collect::<Result<Vec<CompressionType>, SubgraphExecutorError>>()?;
            Ok(CompressionType::Multiple(types))
        } else {
            match encodings[0].to_lowercase().as_str() {
                "gzip" => Ok(CompressionType::Gzip),
                "deflate" => Ok(CompressionType::Deflate),
                "br" => Ok(CompressionType::Brotli),
                "zstd" => Ok(CompressionType::Zstd),
                "identity" => Ok(CompressionType::Identity),
                "none" => Ok(CompressionType::Identity),
                _ => Err(SubgraphExecutorError::UnknownEncoding(encoding.to_string())),
            }
        }
    }
    pub fn decompress<'a>(
        &'a self,
        body: Bytes,
    ) -> BoxFuture<'a, Result<Bytes, SubgraphExecutorError>> {
        Box::pin(async move {
            match self {
                CompressionType::Gzip => {
                    let mut decoder = GzipDecoder::new(body.as_ref());
                    let mut buf = Vec::new();
                    let _ = decoder.read_to_end(&mut buf).await.map_err(|e| {
                        SubgraphExecutorError::DecompressionFailed(
                            self.header_value(),
                            e.to_string(),
                        )
                    });
                    Ok(Bytes::from(buf))
                }
                CompressionType::Deflate => {
                    let mut decoder = DeflateDecoder::new(body.as_ref());
                    let mut buf = Vec::new();
                    let _ = decoder.read_to_end(&mut buf).await.map_err(|e| {
                        SubgraphExecutorError::DecompressionFailed(
                            self.header_value(),
                            e.to_string(),
                        )
                    });
                    Ok(Bytes::from(buf))
                }
                CompressionType::Brotli => {
                    let mut decoder = BrotliDecoder::new(body.as_ref());
                    let mut buf = Vec::new();
                    let _ = decoder.read_to_end(&mut buf).await.map_err(|e| {
                        SubgraphExecutorError::DecompressionFailed(
                            self.header_value(),
                            e.to_string(),
                        )
                    });
                    Ok(Bytes::from(buf))
                }
                CompressionType::Zstd => {
                    let mut decoder = ZstdDecoder::new(body.as_ref());
                    let mut buf = Vec::new();
                    let _ = decoder.read_to_end(&mut buf).await.map_err(|e| {
                        SubgraphExecutorError::DecompressionFailed(
                            self.header_value(),
                            e.to_string(),
                        )
                    });
                    Ok(Bytes::from(buf))
                }
                CompressionType::Multiple(types) => {
                    let mut decompressed_body = body;
                    for ctype in types {
                        decompressed_body = ctype.decompress(decompressed_body).await?;
                    }
                    Ok(decompressed_body)
                }
                CompressionType::Identity => Ok(body),
            }
        })
    }
}
