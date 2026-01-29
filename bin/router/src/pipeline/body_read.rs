use futures::TryStreamExt;
use http::header::CONTENT_LENGTH;
use ntex::{
    util::{Bytes, BytesMut},
    web::{self, HttpRequest},
};

use crate::{pipeline::error::PipelineError, RouterSharedState};

#[inline]
pub async fn read_body_stream(
    req: &HttpRequest,
    mut body_stream: web::types::Payload,
    shared_state: &RouterSharedState,
) -> Result<Bytes, PipelineError> {
    let max_size = shared_state
        .router_config
        .limits
        .max_request_body_size
        .to_bytes() as usize;

    let content_length_header = req.headers().get(CONTENT_LENGTH);
    if let Some(content_length_header) = content_length_header {
        let content_length_str = content_length_header
            .to_str()
            .map_err(|_| PipelineError::InvalidHeaderValue(CONTENT_LENGTH))?;
        let content_length: usize = content_length_str
            .parse()
            .map_err(|_| PipelineError::InvalidHeaderValue(CONTENT_LENGTH))?;
        if content_length > max_size {
            return Err(PipelineError::PayloadTooLarge);
        }
    }

    let mut body = BytesMut::new();
    while let Some(chunk) = body_stream.try_next().await? {
        // limit max size of in-memory payload
        if chunk.len() > max_size.saturating_sub(body.len()) {
            return Err(PipelineError::PayloadTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}
