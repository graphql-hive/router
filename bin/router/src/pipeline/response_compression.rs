use std::{
    error::Error,
    io::Write,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
};

use http::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE};
use ntex::{
    http::{
        body::{Body, BodySize, MessageBody, ResponseBody},
        encoding::Encoder,
        header::{ContentEncoding, HeaderValue},
        StatusCode,
    },
    service::{Ctx, Service},
    util::{Bytes, BytesMut},
    web,
    Middleware,
};
use tokio_stream::StreamExt;
use tracing::error;

use crate::{
    config::traffic_shaping::{
        BrotliCompressionConfig, CompressionAlgorithmConfig, TrafficShapingRouterCompressionConfig,
        ZstdCompressionConfig,
    },
    executor::{execution::plan::FailedExecutionResult, response::graphql_error::GraphQLError},
    http_utils::headers::append_vary,
    pipeline::error::{InternalPipelineError, PipelineError},
    telemetry::logging::{summary::RequestSummary, targets},
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

impl<S, St> Middleware<S, St> for ResponseCompressionService {
    type Service = ResponseCompressionMiddleware<S>;

    fn create(&self, _st: &St, service: S) -> Self::Service {
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

impl<S, St> Service<St, web::WebRequest<()>> for ResponseCompressionMiddleware<S>
where
    S: Service<St, web::WebRequest<()>, Res = web::WebResponse>,
{
    type Res = web::WebResponse;
    type Error = S::Error;

    ntex::forward_bind!(St, service);
    ntex::forward_ready!(St, service);
    ntex::forward_shutdown!(St, service);

    async fn call(
        &self,
        req: web::WebRequest<()>,
        ctx: Ctx<'_, Self, St>,
    ) -> Result<Self::Res, Self::Error> {
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
        let body_size = response.response().body().size();
        let should_compress = match body_size {
            BodySize::Sized(n) => n >= min_size,
            // SSE or unknown stream size are not compressible
            BodySize::Stream => false,
            BodySize::Empty => false,
            _ => false,
        };

        if !should_compress {
            return Ok(response);
        }

        let summary = response
            .request()
            .extensions()
            .get::<Arc<RequestSummary>>()
            .cloned();
        let token = algorithm.token();

        if let Some(summary) = &summary {
            summary.set_response_compression(token);
        }

        let response = match algorithm {
            // ntex's streaming `Encoder` leaves sized bodies below 1KiB untouched, so those are
            // buffered and compressed here to honour a configured `min_size` below that floor
            CompressionAlgorithmConfig::Gzip | CompressionAlgorithmConfig::Deflate
                if matches!(body_size, BodySize::Sized(n) if n < NTEX_ENCODER_MIN_SIZE) =>
            {
                // compressing under 1KiB takes microseconds, way less than a blocking pool hop.
                // the fast level matches what ntex's encoder uses for the larger bodies
                let gzip = matches!(algorithm, CompressionAlgorithmConfig::Gzip);
                compress_full_body(response, token, false, flate_fast_compressor(gzip)).await
            }
            CompressionAlgorithmConfig::Gzip => response.map_body(move |head, body| {
                track_response_bytes(
                    Encoder::response(ContentEncoding::Gzip, head, body),
                    summary,
                )
            }),
            CompressionAlgorithmConfig::Deflate => response.map_body(move |head, body| {
                track_response_bytes(
                    Encoder::response(ContentEncoding::Deflate, head, body),
                    summary,
                )
            }),
            // br/zstd buffer the whole body; `compress_full_body` records the compressed size once
            // compression succeeds.
            CompressionAlgorithmConfig::Br(brotli) => {
                compress_full_body(response, token, true, brotli_compressor(*brotli)).await
            }
            CompressionAlgorithmConfig::Zstd(zstd) => {
                compress_full_body(response, token, true, zstd_compressor(*zstd)).await
            }
        };

        Ok(response)
    }
}

/// Smallest sized body that ntex's streaming `Encoder::response` compresses, smaller bodies are
/// passed through unchanged. Mirrors the private `MIN_SIZE` constant in ntex's encoder.
const NTEX_ENCODER_MIN_SIZE: u64 = 1024;

/// Picks the algorithm from `algorithms` that best matches the client's `Accept-Encoding` preference
fn negotiate<'cfg>(
    accept_encoding: Option<&HeaderValue>,
    algorithms: &'cfg [CompressionAlgorithmConfig],
) -> Option<&'cfg CompressionAlgorithmConfig> {
    let header = accept_encoding?.to_str().ok()?;

    // Keep `q=0` entries in here (rather than dropping them) - per RFC 9110 §12.5.3, `*`
    // only matches codings that aren't explicitly named elsewhere in the field, and an
    // explicit `gzip;q=0` still counts as "named" even though it's not itself acceptable.
    let candidates: Vec<(&str, f64)> = header
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

    let highest_quality = |matches: &dyn Fn(&str) -> bool| {
        candidates
            .iter()
            .filter(|(token, _)| matches(token))
            .map(|(_, quality)| *quality)
            .reduce(f64::max)
    };
    let wildcard_quality = highest_quality(&|token| token == "*");

    // RFC 9110 §12.5.3 gives the listed order of codings no meaning, so equally acceptable
    // algorithms are ranked by the order of `algorithms` rather than by the client's order
    let mut best: Option<(&CompressionAlgorithmConfig, f64)> = None;
    for algorithm in algorithms {
        let quality =
            highest_quality(&|token| token != "*" && token.eq_ignore_ascii_case(algorithm.token()))
                .or(wildcard_quality)
                .unwrap_or(0.0);
        if quality > 0.0 && best.is_none_or(|(_, best_quality)| quality > best_quality) {
            best = Some((algorithm, quality));
        }
    }

    best.map(|(algorithm, _)| algorithm)
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

/// Compresses with gzip, or zlib-wrapped deflate when `gzip` is false, at the fast level
/// that ntex's streaming `Encoder` uses, so small and large bodies compress alike.
fn flate_fast_compressor(gzip: bool) -> impl FnOnce(&[u8]) -> Option<Vec<u8>> {
    move |data| {
        let level = flate2::Compression::fast();
        if gzip {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), level);
            encoder.write_all(data).ok()?;
            encoder.finish().ok()
        } else {
            let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), level);
            encoder.write_all(data).ok()?;
            encoder.finish().ok()
        }
    }
}

fn zstd_compressor(config: ZstdCompressionConfig) -> impl FnOnce(&[u8]) -> Option<Vec<u8>> {
    move |data| zstd::stream::encode_all(data, config.level).ok()
}

/// Drains the response body, runs `compress` on a blocking-pool thread when `offload` is set
/// or on the current thread otherwise, and rebuilds the response with the compressed bytes
/// and a `Content-Encoding` header
///
/// Compression is best-effort: if it fails, we log the failure/error and return the original response
async fn compress_full_body(
    response: web::WebResponse,
    token: &'static str,
    offload: bool,
    compress: impl FnOnce(&[u8]) -> Option<Vec<u8>> + Send + 'static,
) -> web::WebResponse {
    let (http_response, request) = response.into_parts();
    let (mut head, mut body) = http_response.into_parts();

    let original = match drain_body(&mut body).await {
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

            let error_response = web::HttpResponse::builder(StatusCode::INTERNAL_SERVER_ERROR)
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
    let compressed = if offload {
        // a failed blocking task counts as a failed compression
        ntex::rt::spawn_blocking(move || compress(&original))
            .await
            .ok()
            .flatten()
    } else {
        compress(&original)
    };
    let compressed_body = match compressed {
        Some(compressed) => {
            head.headers_mut()
                .insert(CONTENT_ENCODING, HeaderValue::from_static(token));

            if let Some(summary) = request.extensions().get::<Arc<RequestSummary>>() {
                summary.set_response_bytes(compressed.len() as i64);
            }

            Body::Bytes(crate::utils::vec_into_ntex_bytes(compressed))
        }
        None => Body::Bytes(fallback),
    };

    web::WebResponse::new(head.set_body(compressed_body), request)
}

async fn drain_body(body: &mut ResponseBody<Body>) -> Result<Bytes, Rc<dyn std::error::Error>> {
    let mut buf = BytesMut::new();
    while let Some(chunk) = body.try_next().await? {
        buf.extend_from_slice(&chunk);
    }
    Ok(buf.freeze())
}

fn track_response_bytes(
    body: ResponseBody<Body>,
    summary: Option<Arc<RequestSummary>>,
) -> ResponseBody<Body> {
    match summary {
        Some(summary) => ResponseBody::Body(Body::from_message(ResponseBytesTrackedBody {
            body,
            summary,
            response_bytes: 0,
        })),
        None => body,
    }
}

struct ResponseBytesTrackedBody {
    body: ResponseBody<Body>,
    summary: Arc<RequestSummary>,
    response_bytes: i64,
}

impl MessageBody for ResponseBytesTrackedBody {
    fn size(&self) -> BodySize {
        self.body.size()
    }

    fn poll_next_chunk(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Rc<dyn Error>>>> {
        let poll = self.body.poll_next_chunk(cx);
        if let Poll::Ready(Some(Ok(chunk))) = &poll {
            self.response_bytes = self.response_bytes.saturating_add(chunk.len() as i64);
        }
        poll
    }
}

impl Drop for ResponseBytesTrackedBody {
    fn drop(&mut self) {
        self.summary.set_response_bytes(self.response_bytes);
    }
}
