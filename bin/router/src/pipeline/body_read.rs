use futures::TryStreamExt;
use http::header::CONTENT_LENGTH;
use ntex::{
    http::error::PayloadError,
    util::{Bytes, BytesMut},
    web::{self, HttpRequest},
};
use strum::IntoStaticStr;

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
}

impl ReadBodyStreamError {
    pub fn status_code(&self) -> http::StatusCode {
        match self {
            Self::PayloadReadError(_) => http::StatusCode::UNPROCESSABLE_ENTITY,
            Self::InvalidContentLengthHeader => http::StatusCode::BAD_REQUEST,
            Self::PayloadTooLargeContentLength(_) | Self::PayloadTooLargeBodyStream => {
                http::StatusCode::PAYLOAD_TOO_LARGE
            }
        }
    }

    pub fn error_code(&self) -> &'static str {
        self.into()
    }
}

#[inline]
pub async fn read_body_stream(
    req: &HttpRequest,
    mut body_stream: web::types::Payload,
    max_size: usize,
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
                return Err(ReadBodyStreamError::PayloadTooLargeContentLength(max_size));
            }
            Some(content_length)
        } else {
            None
        }
    };

    let mut body = if let Some(content_length) = content_length {
        BytesMut::with_capacity(content_length)
    } else {
        BytesMut::new()
    };

    while let Some(chunk) = body_stream.try_next().await? {
        // limit max size of in-memory payload
        if chunk.len() > max_size.saturating_sub(body.len()) {
            return Err(ReadBodyStreamError::PayloadTooLargeBodyStream);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}
