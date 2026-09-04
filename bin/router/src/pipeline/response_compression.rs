use std::{cmp::Ordering, io::Write, rc::Rc};

use http::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE};
use ntex::{
    http::{
        body::{Body, BodySize, MessageBody, ResponseBody},
        encoding::Encoder,
        header::{ContentEncoding, HeaderValue},
        StatusCode,
    },
    service::{Service, ServiceCtx},
    util::{Bytes, BytesMut},
    web::{self, DefaultError},
    Middleware, SharedCfg,
};
use tokio_stream::StreamExt;
use tracing::error;

use crate::{
    config::traffic_shaping::{
        BrotliCompressionConfig, ResponseCompressionAlgorithmConfig,
        TrafficShapingRouterCompressionConfig, ZstdCompressionConfig,
    },
    executor::{execution::plan::FailedExecutionResult, response::graphql_error::GraphQLError},
    http_utils::headers::append_vary,
    pipeline::error::{InternalPipelineError, PipelineError},
    telemetry::logging::targets,
};

#[derive(Clone)]
pub struct ResponseCompressionService {
    response_compression_config: &'static TrafficShapingRouterCompressionConfig,
}

impl ResponseCompressionService {
    pub fn new(router_config: &'static crate::config::HiveRouterConfig) -> Self {
        Self {
            response_compression_config: &router_config.traffic_shaping.router.compression,
        }
    }
}

impl<S> Middleware<S, SharedCfg> for ResponseCompressionService {
    type Service = ResponseCompressionMiddleware<S>;

    fn create(&self, service: S, _cfg: SharedCfg) -> Self::Service {
        ResponseCompressionMiddleware {
            service,
            response_compression_config: self.response_compression_config,
        }
    }
}

pub struct ResponseCompressionMiddleware<S> {
    service: S,
    response_compression_config: &'static TrafficShapingRouterCompressionConfig,
}

impl<S> Service<web::WebRequest<DefaultError>> for ResponseCompressionMiddleware<S>
where
    S: Service<web::WebRequest<DefaultError>, Response = web::WebResponse, Error = web::Error>,
{
    type Response = web::WebResponse;
    type Error = S::Error;

    ntex::forward_ready!(service);

    async fn call(
        &self,
        req: web::WebRequest<DefaultError>,
        ctx: ServiceCtx<'_, Self>,
    ) -> Result<Self::Response, Self::Error> {
        let config = &self.response_compression_config.response;

        if !config.enabled {
            return ctx.call(&self.service, req).await;
        }

        let negotiated = negotiate(req.headers().get(&ACCEPT_ENCODING), &config.algorithms);
        let mut response = ctx.call(&self.service, req).await?;

        // An indicator for caching/proxy layers that the response depends on Accept-Encoding
        // and that compression was applied to it.
        append_vary(response.headers_mut(), ACCEPT_ENCODING.as_str());

        let Some(algorithm) = negotiated else {
            return Ok(response);
        };

        let min_size = config.min_size.to_bytes();
        let should_compress = match response.response().body().size() {
            BodySize::Sized(n) => n >= min_size,
            // SSE or unknown stream size are not compressible
            BodySize::Stream => false,
            BodySize::Empty => false,
            _ => false,
        };

        if !should_compress {
            return Ok(response);
        }

        let response = match algorithm {
            ResponseCompressionAlgorithmConfig::Gzip => {
                response.map_body(|head, body| Encoder::response(ContentEncoding::Gzip, head, body))
            }
            ResponseCompressionAlgorithmConfig::Deflate => response
                .map_body(|head, body| Encoder::response(ContentEncoding::Deflate, head, body)),
            ResponseCompressionAlgorithmConfig::Br(brotli) => {
                compress_full_body(response, "br", brotli_compressor(*brotli)).await
            }
            ResponseCompressionAlgorithmConfig::Zstd(zstd) => {
                compress_full_body(response, "zstd", zstd_compressor(*zstd)).await
            }
        };

        Ok(response)
    }
}

/// Picks the algorithm from `algorithms` that best matches the client's `Accept-Encoding` preference
fn negotiate<'cfg>(
    accept_encoding: Option<&HeaderValue>,
    algorithms: &'cfg [ResponseCompressionAlgorithmConfig],
) -> Option<&'cfg ResponseCompressionAlgorithmConfig> {
    let header = accept_encoding?.to_str().ok()?;

    // Keep `q=0` entries in here (rather than dropping them) - per RFC 9110 §12.5.3, `*`
    // only matches codings that aren't explicitly named elsewhere in the field, and an
    // explicit `gzip;q=0` still counts as "named" even though it's not itself acceptable.
    let mut candidates: Vec<(&str, f64)> = header
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let mut segments = part.split(';');
            let token = segments.next()?.trim();
            let quality = segments
                .find_map(|seg| seg.trim().strip_prefix("q=")?.trim().parse::<f64>().ok())
                .unwrap_or(1.0);
            Some((token, quality))
        })
        .collect();

    // stable sort by quality: ties keep the client's original listed order
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

    let is_explicitly_named = |token: &str| {
        candidates
            .iter()
            .any(|(t, _)| *t != "*" && t.eq_ignore_ascii_case(token))
    };

    for (token, quality) in &candidates {
        if *quality <= 0.0 {
            continue;
        }
        if *token == "*" {
            if let Some(algorithm) = algorithms.iter().find(|a| !is_explicitly_named(a.token())) {
                return Some(algorithm);
            }
            continue;
        }
        if let Some(algorithm) = algorithms
            .iter()
            .find(|a| a.token().eq_ignore_ascii_case(token))
        {
            return Some(algorithm);
        }
    }

    None
}

fn brotli_compressor(config: BrotliCompressionConfig) -> impl FnOnce(&[u8]) -> Option<Vec<u8>> {
    move |data| {
        let mut out = Vec::new();
        let mut writer = brotli::CompressorWriter::new(&mut out, 4096, config.quality as u32, 22);
        writer.write_all(data).ok()?;
        drop(writer);
        Some(out)
    }
}

fn zstd_compressor(config: ZstdCompressionConfig) -> impl FnOnce(&[u8]) -> Option<Vec<u8>> {
    move |data| zstd::stream::encode_all(data, config.level).ok()
}

/// Drains the response body, runs `compress` on a blocking-pool thread, and rebuilds the
/// response with the compressed bytes and a `Content-Encoding` header
///
/// Compression is best-effort: if it fails, we log the failure/error and return the original response
async fn compress_full_body(
    response: web::WebResponse,
    token: &'static str,
    compress: impl FnOnce(&[u8]) -> Option<Vec<u8>> + Send + 'static,
) -> web::WebResponse {
    let (http_response, request) = response.into_parts();
    let (mut head, body) = http_response.into_parts();

    let original = match drain_body(body).await {
        Ok(bytes) => bytes,
        Err(err) => {
            let pipeline_err: PipelineError =
                InternalPipelineError::ResponseCompressionFailed(err.to_string()).into();

            error!(
                target: targets::HTTP_SERVER,
                error = %pipeline_err,
                "failed to read response body while compressing it; the body stream broke \
                 mid-read and the original response can't be recovered"
            );

            let error_response = web::HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR)
                .header(CONTENT_TYPE, "application/json")
                .body(
                    FailedExecutionResult {
                        errors: vec![GraphQLError::from_message_and_code(
                            pipeline_err.graphql_error_message(),
                            pipeline_err.graphql_error_code(),
                        )],
                    }
                    .serialize(),
                );

            return web::WebResponse::new(error_response, request);
        }
    };

    let fallback = original.clone();
    let body = match ntex::rt::spawn_blocking(move || compress(&original)).await {
        Ok(Some(compressed)) => {
            head.headers_mut()
                .insert(CONTENT_ENCODING, HeaderValue::from_static(token));
            Body::Bytes(Bytes::from(compressed))
        }
        _ => Body::Bytes(fallback),
    };

    web::WebResponse::new(head.set_body(body), request)
}

async fn drain_body(mut body: ResponseBody<Body>) -> Result<Bytes, Rc<dyn std::error::Error>> {
    let mut buf = BytesMut::new();
    while let Some(chunk) = body.try_next().await? {
        buf.extend_from_slice(&chunk);
    }
    Ok(buf.freeze())
}
