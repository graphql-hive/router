use bytes::Bytes;
use hive_router_config::coprocessor::{
    CoprocessorConfig, CoprocessorEndpoint, CoprocessorProtocol,
};
use hive_router_internal::telemetry::metrics::catalog::values::GraphQLResponseStatus;
use hive_router_internal::telemetry::traces::spans::http_request::HttpClientRequestSpan;
use hive_router_internal::telemetry::TelemetryContext;
use http::{HeaderMap, HeaderValue, Method, Request, Response, Uri};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::client::legacy::{connect::HttpConnector, Client as HyperClient};
use hyper_util::rt::{TokioExecutor, TokioTimer};
use hyperlocal::{UnixConnector, Uri as HyperlocalUri};
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use tracing::Instrument;

use super::error::CoprocessorError;

type HttpClient = HyperClient<HttpConnector, Full<Bytes>>;
type UnixClient = HyperClient<UnixConnector, Full<Bytes>>;

const ACCEPT_ENCODING_VALUE: HeaderValue = HeaderValue::from_static("gzip, br, deflate");
const CONTENT_TYPE_VALUE: HeaderValue = HeaderValue::from_static("application/json");

enum Client {
    Http { client: Arc<HttpClient> },
    Unix { client: Arc<UnixClient> },
}

enum Compression {
    GZip,
    Brotli,
    Deflate,
    None,
}

impl Compression {
    pub fn decompress(&self, body: Bytes) -> Result<Bytes, CoprocessorError> {
        match self {
            Compression::None => Ok(body),
            Compression::GZip => Self::gzip(body),
            Compression::Brotli => Self::brotli(body),
            Compression::Deflate => Self::deflate(body),
        }
    }

    fn decompress_with<R: Read>(
        mut decoder: R,
        capacity: usize,
        encoding: &'static str,
    ) -> Result<Bytes, CoprocessorError> {
        let mut out = Vec::with_capacity(capacity * 4);
        decoder.read_to_end(&mut out).map_err(|source| {
            CoprocessorError::ResponseDecompressionFailure { encoding, source }
        })?;

        Ok(Bytes::from(out))
    }

    fn gzip(body: Bytes) -> Result<Bytes, CoprocessorError> {
        let decoder = flate2::read::GzDecoder::new(body.as_ref());
        Self::decompress_with(decoder, body.len(), "gzip")
    }

    fn brotli(body: Bytes) -> Result<Bytes, CoprocessorError> {
        let decoder = brotli::Decompressor::new(body.as_ref(), 4096);
        Self::decompress_with(decoder, body.len(), "br")
    }

    fn deflate(body: Bytes) -> Result<Bytes, CoprocessorError> {
        let decoder = flate2::read::ZlibDecoder::new(body.as_ref());
        Self::decompress_with(decoder, body.len(), "deflate")
    }
}

impl TryFrom<&HeaderMap> for Compression {
    type Error = CoprocessorError;

    fn try_from(headers: &HeaderMap) -> Result<Self, Self::Error> {
        let Some(content_encoding) = headers.get(http::header::CONTENT_ENCODING) else {
            return Ok(Compression::None);
        };

        let content_encoding = content_encoding
            .to_str()
            .map_err(CoprocessorError::InvalidContentEncodingHeader)?
            .trim();

        if content_encoding.is_empty() || content_encoding.eq_ignore_ascii_case("identity") {
            return Ok(Compression::None);
        }

        if content_encoding.as_bytes().contains(&b',') {
            // We intentionally reject multi encodings.
            // Receiving `gzip, deflate` and having to decompress with `deflate` and then `gzip`
            // is super weird and very very rare.
            return Err(CoprocessorError::UnsupportedStackedContentEncoding(
                content_encoding.to_string(),
            ));
        }

        if content_encoding.eq_ignore_ascii_case("gzip") {
            return Ok(Compression::GZip);
        }

        if content_encoding.eq_ignore_ascii_case("br") {
            return Ok(Compression::Brotli);
        }

        if content_encoding.eq_ignore_ascii_case("deflate") {
            return Ok(Compression::Deflate);
        }

        Err(CoprocessorError::UnsupportedContentEncoding(
            content_encoding.to_string(),
        ))
    }
}

impl Client {
    async fn request(
        &self,
        request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, CoprocessorError> {
        let res = match self {
            Client::Http { client } => client
                .request(request)
                .await
                .map_err(CoprocessorError::RequestExecutionFailure)?,
            Client::Unix { client } => client
                .request(request)
                .await
                .map_err(CoprocessorError::RequestExecutionFailure)?,
        };

        Ok(res)
    }
}

pub struct CoprocessorClient {
    client: Client,
    endpoint: Uri,
    timeout: Duration,
    telemetry_context: Arc<TelemetryContext>,
}

impl CoprocessorClient {
    pub fn new(
        config: CoprocessorConfig,
        telemetry_context: Arc<TelemetryContext>,
    ) -> Result<Self, CoprocessorError> {
        let timeout = config.timeout;

        match (&config.url, config.protocol) {
            (CoprocessorEndpoint::Http { url }, protocol) => {
                let endpoint = url
                    .parse::<Uri>()
                    .map_err(|error| CoprocessorError::EndpointParseFailure(url.clone(), error))?;

                let client = Arc::new(build_http_client(protocol)?);

                Ok(Self {
                    client: Client::Http { client },
                    endpoint,
                    timeout,
                    telemetry_context,
                })
            }
            (
                CoprocessorEndpoint::Unix {
                    socket_path,
                    request_path,
                },
                protocol,
            ) => {
                if !request_path.starts_with('/') {
                    return Err(CoprocessorError::InvalidUnixRequestPath(
                        request_path.clone(),
                    ));
                }

                let endpoint: Uri = HyperlocalUri::new(socket_path, request_path.as_str()).into();
                let client = Arc::new(build_unix_client(protocol)?);

                Ok(Self {
                    client: Client::Unix { client },
                    endpoint,
                    timeout,
                    telemetry_context,
                })
            }
        }
    }

    pub async fn send(&self, body: Bytes) -> Result<Response<Bytes>, CoprocessorError> {
        let request_body_size = body.len() as u64;
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(&self.endpoint)
            .header(http::header::CONTENT_TYPE, CONTENT_TYPE_VALUE)
            .header(http::header::ACCEPT_ENCODING, ACCEPT_ENCODING_VALUE)
            .body(Full::new(body))
            .map_err(CoprocessorError::RequestBuildFailure)?;

        let mut request_capture = self.telemetry_context.metrics.http_client.capture_request(
            &request,
            request_body_size,
            None,
        );
        let http_request_span = HttpClientRequestSpan::from_request(&request);

        let response = async {
            // Forward trace context so coprocessor spans/logs can be correlated.
            self.telemetry_context
                .inject_context_into_http_headers(request.headers_mut());

            let start = std::time::Instant::now();
            let response = tokio::time::timeout(self.timeout, self.client.request(request))
                .await
                .map_err(|_| CoprocessorError::RequestTimeout {
                    endpoint: self.endpoint.to_string(),
                    timeout_ms: self.timeout.as_millis(),
                });

            let response = match response {
                Ok(Ok(res)) => res,
                Ok(Err(err)) => {
                    request_capture.finish_error(err.error_code(), start.elapsed());
                    return Err(err);
                }
                Err(err) => {
                    request_capture.finish_error(err.error_code(), start.elapsed());
                    return Err(err);
                }
            };

            http_request_span.record_response(&response);
            request_capture.set_status_code(response.status().as_u16());

            if !response.status().is_success() {
                let error = CoprocessorError::UnexpectedStatus(response.status());
                request_capture.finish(
                    0,
                    start.elapsed(),
                    GraphQLResponseStatus::Error,
                    Some(error.error_code()),
                );
                return Err(error);
            }

            let (parts, response_body) = response.into_parts();
            let response_body = match response_body.collect().await {
                Ok(body) => body.to_bytes(),
                Err(err) => {
                    let error = CoprocessorError::ResponseBodyReadFailure(err);
                    request_capture.finish_error(error.error_code(), start.elapsed());
                    return Err(error);
                }
            };

            request_capture.finish(
                response_body.len() as u64,
                start.elapsed(),
                GraphQLResponseStatus::Ok,
                None,
            );

            let compression = Compression::try_from(&parts.headers)?;
            let decompressed = compression.decompress(response_body)?;

            Ok(Response::from_parts(parts, decompressed))
        }
        .instrument(http_request_span.clone())
        .await;

        if response.is_err() {
            http_request_span.record_internal_server_error();
        }

        response
    }
}

fn build_http_client(protocol: CoprocessorProtocol) -> Result<HttpClient, CoprocessorError> {
    // `http2` config value is reserved for future (https) and explicitly rejected for now.
    if protocol == CoprocessorProtocol::Http2 {
        return Err(CoprocessorError::UnsupportedProtocol(protocol));
    }

    let mut connector = HttpConnector::new();
    connector.enforce_http(true);
    connector.set_keepalive(Some(Duration::from_secs(60)));

    let builder = client_builder(protocol);

    Ok(builder.build(connector))
}

fn build_unix_client(protocol: CoprocessorProtocol) -> Result<UnixClient, CoprocessorError> {
    // `http2` config value is reserved for future (https) and explicitly rejected for now.
    if protocol == CoprocessorProtocol::Http2 {
        return Err(CoprocessorError::UnsupportedProtocol(protocol));
    }

    let connector = UnixConnector;
    let builder = client_builder(protocol);

    Ok(builder.build(connector))
}

fn client_builder(protocol: CoprocessorProtocol) -> hyper_util::client::legacy::Builder {
    let mut builder = HyperClient::builder(TokioExecutor::new());
    builder.pool_timer(TokioTimer::new());
    builder.pool_idle_timeout(Duration::from_secs(60));

    if protocol == CoprocessorProtocol::H2c {
        builder.http2_only(true);
    }

    builder
}
