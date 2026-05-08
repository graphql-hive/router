use std::cell::{Ref, RefMut};

use futures::TryStreamExt;
use http::{header::CONTENT_LENGTH, uri::Scheme, Method, Uri, Version};
use ntex::{
    http::{error::PayloadError, HeaderMap},
    util::{Bytes, BytesMut, Extensions},
    web::{self, DefaultError, HttpRequest, WebRequest},
};
use strum::IntoStaticStr;

pub trait HttpUriAsStr {
    fn scheme_static_str(&self) -> &'static str;
}

impl HttpUriAsStr for Uri {
    fn scheme_static_str(&self) -> &'static str {
        if self.scheme() == Some(&Scheme::HTTPS) {
            "https"
        } else {
            "http"
        }
    }
}

pub trait HttpVersionAsStr {
    fn as_static_str(&self) -> &'static str;
}

impl HttpVersionAsStr for Version {
    fn as_static_str(&self) -> &'static str {
        match *self {
            Version::HTTP_09 => "0.9",
            Version::HTTP_10 => "1.0",
            Version::HTTP_11 => "1.1",
            Version::HTTP_2 => "2",
            Version::HTTP_3 => "3",
            // SAFETY: only supported HTTP versions will ever reach router
            _ => unreachable!("Unknown HTTP version"),
        }
    }
}

pub trait HttpMethodAsStr {
    fn as_static_str(&self) -> &'static str;
}

impl HttpMethodAsStr for Method {
    fn as_static_str(&self) -> &'static str {
        match *self {
            Method::GET => "GET",
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::PATCH => "PATCH",
            Method::DELETE => "DELETE",
            Method::HEAD => "HEAD",
            Method::OPTIONS => "OPTIONS",
            Method::CONNECT => "CONNECT",
            Method::TRACE => "TRACE",
            _ => {
                if self.as_str() == "QUERY" {
                    // Special case for QUERY method,
                    // that is not yet stable
                    "QUERY"
                } else {
                    // For telemetry purposes, we log the method as "_OTHER"
                    "_OTHER"
                }
            }
        }
    }
}

pub fn normalize_route_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

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

pub struct RequestBodyBytes(pub Bytes);

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
fn write_request_body_size<R: RequestLike>(req: &R, size: u64) {
    req.extensions_mut().insert(RequestBodySize(size));
}

#[inline]
pub fn read_request_body_size(req: &HttpRequest) -> Option<u64> {
    req.extensions().get::<RequestBodySize>().map(|size| size.0)
}

#[inline]
pub async fn read_body_stream<R: RequestLike>(
    req: &R,
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
                write_request_body_size(req, content_length as u64);
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
            write_request_body_size(req, (body.len() + chunk.len()) as u64);
            return Err(ReadBodyStreamError::PayloadTooLargeBodyStream);
        }
        body.extend_from_slice(&chunk);
    }

    write_request_body_size(req, body.len() as u64);

    Ok(body.freeze())
}

pub trait RequestLike {
    fn headers(&self) -> &HeaderMap;
    fn extensions(&self) -> Ref<'_, Extensions>;
    fn extensions_mut(&self) -> RefMut<'_, Extensions>;
}

impl RequestLike for HttpRequest {
    fn headers(&self) -> &HeaderMap {
        self.headers()
    }

    fn extensions(&self) -> Ref<'_, Extensions> {
        self.extensions()
    }

    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.extensions_mut()
    }
}

impl RequestLike for WebRequest<DefaultError> {
    fn headers(&self) -> &HeaderMap {
        self.headers()
    }

    fn extensions(&self) -> Ref<'_, Extensions> {
        self.extensions()
    }

    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.extensions_mut()
    }
}
