use http::header::CONTENT_ENCODING;
use ntex::http::HeaderMap;

use crate::{
    config::traffic_shaping::{CompressionAlgorithm, TrafficShapingRouterRequestCompressionConfig},
    pipeline::error::ClientPipelineError,
};

pub fn negotiate_request_encoding(
    headers: &HeaderMap,
    config: &TrafficShapingRouterRequestCompressionConfig,
) -> Result<Option<CompressionAlgorithm>, ClientPipelineError> {
    let Some(header) = headers.get(&CONTENT_ENCODING) else {
        return Ok(None);
    };

    let raw = header
        .to_str()
        .map_err(|_| ClientPipelineError::InvalidHeaderValue(CONTENT_ENCODING))?
        .trim();

    if raw.eq_ignore_ascii_case("identity") {
        return Ok(None);
    }

    if !config.enabled || raw.contains(',') {
        return Err(ClientPipelineError::UnsupportedContentEncoding(
            raw.to_string(),
        ));
    }

    CompressionAlgorithm::from_token(raw)
        .filter(|algorithm| config.algorithms.contains(algorithm))
        .ok_or_else(|| ClientPipelineError::UnsupportedContentEncoding(raw.to_string()))
        .map(Some)
}
