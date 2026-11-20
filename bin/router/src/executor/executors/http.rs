use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::traffic_shaping::{
    AcceptEncodingConfig, ClientTLSConfig, CompressionAlgorithm, HttpClientMode,
    SupergraphTrafficShapingConfig, TrafficShapingSubgraphRequestCompressionConfig,
};
use crate::executor::executors::dedupe::unique_leader_fingerprint;
use crate::executor::executors::inflight::InFlightRole;
use crate::executor::executors::map::InflightRequestsMap;
use crate::executor::executors::multipart_subscribe;
use crate::executor::executors::sse;
use crate::executor::executors::subscription_buffer;
use crate::executor::hooks::on_subgraph_http_request::{
    OnSubgraphHttpRequestHookPayload, OnSubgraphHttpResponseHookPayload,
};
use crate::executor::json_writer::{named_operation_parts, write_named_operation};
use crate::executor::plugin_context::PluginRequestState;
use crate::executor::plugin_trait::{EndControlFlow, StartControlFlow};
use crate::executor::plugins::hooks;
use crate::executor::response::shaped::ResponseShape;
use crate::executor::response::subgraph_response::SubgraphResponse;
use crate::http_utils::compression;
use crate::query_planner::planner::plan_nodes::CustomScalarPaths;
use crate::telemetry::external_wait;
use crate::telemetry::logging::targets;
use crate::telemetry::metrics::catalog::values::GraphQLResponseStatus;
use crate::telemetry::metrics::http_client_metrics::HttpClientRequestStateCapture;
use crate::telemetry::metrics::subscription_metrics::SubscriptionTransport;
use crate::telemetry::TelemetryContext;
use futures::stream::BoxStream;

use async_trait::async_trait;

use bytes::{BufMut, Bytes, BytesMut};
use http::HeaderMap;
use http::HeaderValue;
use http::StatusCode;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::Body as _;
use hyper::Version;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioTimer};

use crate::executor::executors::connection_age::{self, AgedConnector};
use crate::executor::executors::tls::build_https_connector;
use thread_local::ThreadLocal;
use tokio::sync::Semaphore;
use tracing::{debug, trace, warn};

use crate::executor::executors::common::{sha256_hex, SubgraphExecutionRequest};
use crate::executor::executors::error::SubgraphExecutorError;
use crate::executor::utils::consts::CLOSE_BRACE;
use crate::executor::utils::consts::COLON;
use crate::executor::utils::consts::COMMA;
use crate::executor::utils::consts::QUOTE;
use crate::executor::{executors::common::SubgraphExecutor, json_writer::write_and_escape_string};
use crate::telemetry::traces::spans::http_request::HttpClientRequestSpan;
use crate::telemetry::traces::spans::http_request::HttpInflightRequestSpan;
use tracing::Instrument;

pub struct HTTPSubgraphExecutor {
    pub subgraph_name: String,
    pub endpoint: http::Uri,
    pub http_client: Arc<HttpClientProvider>,
    pub header_map: HeaderMap,
    pub semaphore: Arc<Semaphore>,
    pub dedupe_enabled: bool,
    pub in_flight_requests: InflightRequestsMap,
    pub telemetry_context: Arc<TelemetryContext>,
    pub subgraph_buffer_capacity: usize,
    pub compression: TrafficShapingSubgraphRequestCompressionConfig,
    apq: SubgraphApq,
}

/// How long APQ stays off after a subgraph says it doesn't support it.
const APQ_COOLDOWN: Duration = Duration::from_secs(5 * 60);

struct SubgraphApq {
    configured: bool,
    /// Until when APQ is off, in ms since `since`. 0 when it's on.
    off_until: AtomicU64,
    since: Instant,
}

impl SubgraphApq {
    fn new(configured: bool) -> Self {
        Self {
            configured,
            off_until: AtomicU64::new(0),
            since: Instant::now(),
        }
    }

    fn should_send_hash(&self) -> bool {
        if !self.configured {
            return false;
        }
        let off_until = self.off_until.load(Ordering::Relaxed);
        off_until == 0 || self.cooldown_over(off_until, self.now_ms())
    }

    /// Returns whether this started a new cooldown, so a burst of rejections logs once.
    fn record_not_supported(&self) -> bool {
        self.record_not_supported_at(self.now_ms())
    }

    fn cooldown_over(&self, off_until: u64, now_ms: u64) -> bool {
        if now_ms < off_until {
            return false;
        }
        // `Err(0)`: someone else cleared it. Any other `Err`: someone renewed it.
        matches!(
            self.off_until
                .compare_exchange(off_until, 0, Ordering::Relaxed, Ordering::Relaxed),
            Ok(_) | Err(0)
        )
    }

    fn record_not_supported_at(&self, now_ms: u64) -> bool {
        let previous = self
            .off_until
            .fetch_max(now_ms + APQ_COOLDOWN.as_millis() as u64, Ordering::Relaxed);
        previous <= now_ms
    }

    fn now_ms(&self) -> u64 {
        self.since.elapsed().as_millis() as u64
    }
}

const QUERY_STR: &[u8] = b"\"query\":";
const VARIABLES_STR: &[u8] = b"\"variables\":{";
const OPERATION_NAME_STR: &[u8] = b"\"operationName\":";
const EXTENSIONS_STR: &[u8] = b"\"extensions\":{";
const PERSISTED_QUERY_STR: &[u8] = b"\"persistedQuery\":{\"version\":1,\"sha256Hash\":\"";

#[derive(Clone, Copy)]
pub enum Apq<'h> {
    Off,
    HashOnly(&'h str),
    QueryAndHash(&'h str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ApqError {
    NotFound,
    NotSupported,
    /// Failed without an APQ error. If the plain query then works, the subgraph ignores APQ.
    Failed,
}

enum HashOnlyResult<'a> {
    Done(SubgraphResponse<'static>),
    Rejected(SubgraphExecutionRequest<'a>, ApqError),
}

/// SHA-256 of the query as sent, operation name included.
fn query_hash<'a>(execution_request: &SubgraphExecutionRequest<'a>) -> Cow<'a, str> {
    match (
        &execution_request.operation_name,
        execution_request.query_hash,
    ) {
        // a plugin may have swapped the query, so the cache only counts for the plan's own one
        (None, Some((plan_query, cached))) if std::ptr::eq(plan_query, execution_request.query) => {
            Cow::Borrowed(cached.get(plan_query))
        }
        (None, _) => Cow::Owned(sha256_hex([execution_request.query])),
        (Some(name), _) => Cow::Owned(sha256_hex(named_operation_parts(
            name,
            execution_request.document_name_write_pos,
            execution_request.query,
        ))),
    }
}

/// An APQ error, but only if nothing ran (no data, no error path).
fn apq_rejection(response: &SubgraphResponse<'_>) -> Option<ApqError> {
    if !response.data.is_null()
        || response
            .errors
            .iter()
            .flatten()
            .any(|error| error.path.is_some())
    {
        return None;
    }
    let errors = response.errors.as_deref().unwrap_or_default();
    let apq_error = errors.iter().find_map(|error| {
        match (error.extensions.code.as_deref(), error.message.as_str()) {
            (Some("PERSISTED_QUERY_NOT_FOUND"), _) | (_, "PersistedQueryNotFound") => {
                Some(ApqError::NotFound)
            }
            (Some("PERSISTED_QUERY_NOT_SUPPORTED"), _) | (_, "PersistedQueryNotSupported") => {
                Some(ApqError::NotSupported)
            }
            _ => None,
        }
    });
    apq_error.or((!errors.is_empty()).then_some(ApqError::Failed))
}

#[inline]
fn put_field(body: &mut Vec<u8>, needs_comma: &mut bool, field: &[u8]) {
    if *needs_comma {
        body.put(COMMA);
    }
    body.put(field);
    *needs_comma = true;
}

pub type HttpClient = Client<HttpsConnector<AgedConnector>, Full<Bytes>>;

pub struct HttpClientProvider {
    clients: HttpClients,
    /// On when `max_connection_age` is set. Only then do requests pay for the age check.
    retire_old_connections: bool,
}

#[allow(clippy::large_enum_variant)]
enum HttpClients {
    /// One client for all workers.
    Shared(HttpClient),
    /// One client per worker thread.
    PerWorker(WorkerLocalClients),
}

struct WorkerLocalClients {
    builder: hyper_util::client::legacy::Builder,
    connector: HttpsConnector<AgedConnector>,
    clients: ThreadLocal<HttpClient>,
}

impl HttpClientProvider {
    /// Builds the client for subgraph requests. The arguments are the settings a subgraph can
    /// override; everything else comes from `traffic_shaping.all`.
    pub fn new(
        traffic_shaping: &SupergraphTrafficShapingConfig,
        tls_config: Option<&ClientTLSConfig>,
        pool_idle_timeout: Duration,
        http2_only: bool,
    ) -> Result<Self, SubgraphExecutorError> {
        let all = &traffic_shaping.all;
        let mut builder = Client::builder(TokioExecutor::new());
        builder
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(pool_idle_timeout)
            .pool_max_idle_per_host(traffic_shaping.max_connections_per_host)
            .http2_only(http2_only);
        if let Some(interval) = all.http2_keep_alive_interval {
            // HTTP/2 pings need their own timer
            builder
                .timer(TokioTimer::new())
                .http2_keep_alive_interval(interval)
                .http2_keep_alive_while_idle(true);
        }
        let connector =
            build_https_connector(tls_config, all.max_connection_age, all.tcp_keepalive)?;
        let clients = match traffic_shaping.http_client_mode {
            HttpClientMode::Shared => HttpClients::Shared(builder.build(connector)),
            HttpClientMode::PerWorker => HttpClients::PerWorker(WorkerLocalClients {
                builder,
                connector,
                clients: ThreadLocal::new(),
            }),
        };
        Ok(Self {
            clients,
            retire_old_connections: all.max_connection_age.is_some(),
        })
    }

    /// The client for the current thread.
    fn client(&self) -> &HttpClient {
        match &self.clients {
            HttpClients::Shared(client) => client,
            HttpClients::PerWorker(local) => local
                .clients
                .get_or(|| local.builder.build(local.connector.clone())),
        }
    }

    pub async fn request(
        &self,
        req: hyper::Request<Full<Bytes>>,
    ) -> Result<hyper::Response<hyper::body::Incoming>, hyper_util::client::legacy::Error> {
        if self.retire_old_connections {
            connection_age::request_and_retire_old(self.client(), req).await
        } else {
            self.client().request(req).await
        }
    }
}

struct FetchedSubgraphResponse<'a> {
    response: SubgraphHttpResponse,
    http_request_capture: HttpClientRequestStateCapture<'a>,
    transport_duration: Duration,
}

struct HttpRequestTelemetryCapture<'a> {
    capture: HttpClientRequestStateCapture<'a>,
    response_body_size: u64,
    transport_duration: Duration,
}

pub fn build_request_body(
    execution_request: &SubgraphExecutionRequest<'_>,
) -> Result<Vec<u8>, SubgraphExecutorError> {
    build_request_body_with_apq(execution_request, Apq::Off)
}

pub fn build_request_body_with_apq(
    execution_request: &SubgraphExecutionRequest<'_>,
    apq: Apq<'_>,
) -> Result<Vec<u8>, SubgraphExecutorError> {
    let capacity = match apq {
        // no query, so mostly the raw variables
        Apq::HashOnly(_) => {
            256 + execution_request
                .raw_variable_values
                .iter()
                .flatten()
                .map(|(name, value)| name.len() + value.len() + 4)
                .sum::<usize>()
        }
        _ => 4096,
    };
    let mut body = Vec::with_capacity(capacity);
    body.put_u8(b'{');
    let mut needs_comma = false;

    let apq_hash = match apq {
        Apq::Off => None,
        Apq::HashOnly(hash) | Apq::QueryAndHash(hash) => Some(hash),
    };

    if !matches!(apq, Apq::HashOnly(_)) {
        put_field(&mut body, &mut needs_comma, QUERY_STR);
        if let Some(operation_name) = &execution_request.operation_name {
            write_named_operation(
                &mut body,
                operation_name.as_bytes(),
                execution_request.document_name_write_pos,
                execution_request.query,
            );
        } else {
            write_and_escape_string(&mut body, execution_request.query);
        }
    }

    let mut first_variable = true;
    if let Some(variables) = &execution_request.variables {
        for (variable_name, variable_value) in variables {
            if first_variable {
                put_field(&mut body, &mut needs_comma, VARIABLES_STR);
                first_variable = false;
            } else {
                body.put(COMMA);
            }
            body.put(QUOTE);
            body.put(variable_name.as_bytes());
            body.put(QUOTE);
            body.put(COLON);
            sonic_rs::to_writer(&mut body, variable_value).map_err(|err| {
                SubgraphExecutorError::VariablesSerializationFailure(variable_name.to_string(), err)
            })?;
        }
    }
    if let Some(raw_variable_values) = &execution_request.raw_variable_values {
        for (variable_name, variable_value) in raw_variable_values {
            if first_variable {
                put_field(&mut body, &mut needs_comma, VARIABLES_STR);
                first_variable = false;
            } else {
                body.put(COMMA);
            }
            body.put(QUOTE);
            body.put(variable_name.as_bytes());
            body.put(QUOTE);
            body.put(COLON);
            body.extend_from_slice(variable_value);
        }
    }
    // "first_variable" should be still true if there are no variables
    if !first_variable {
        body.put(CLOSE_BRACE);
    }

    if let Some(operation_name) = &execution_request.operation_name {
        put_field(&mut body, &mut needs_comma, OPERATION_NAME_STR);
        body.put(QUOTE);
        body.put(operation_name.as_bytes());
        body.put(QUOTE);
    }

    let mut first_extension = true;
    if let Some(hash) = apq_hash {
        put_field(&mut body, &mut needs_comma, EXTENSIONS_STR);
        first_extension = false;
        body.put(PERSISTED_QUERY_STR);
        body.put(hash.as_bytes());
        body.put(&b"\"}"[..]);
    }
    if let Some(extensions) = &execution_request.extensions {
        for (name, value) in extensions {
            if apq_hash.is_some() && name == "persistedQuery" {
                continue;
            }
            if first_extension {
                put_field(&mut body, &mut needs_comma, EXTENSIONS_STR);
                first_extension = false;
            } else {
                body.put(COMMA);
            }
            write_and_escape_string(&mut body, name);
            body.put(COLON);
            sonic_rs::to_writer(&mut body, value).expect("extension values are JSON values");
        }
    }
    if !first_extension {
        body.put(CLOSE_BRACE);
    }

    body.put(CLOSE_BRACE);

    Ok(body)
}

impl HTTPSubgraphExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        subgraph_name: String,
        endpoint: http::Uri,
        http_client: Arc<HttpClientProvider>,
        semaphore: Arc<Semaphore>,
        dedupe_enabled: bool,
        in_flight_requests: InflightRequestsMap,
        telemetry_context: Arc<TelemetryContext>,
        subgraph_buffer_capacity: usize,
        compression: TrafficShapingSubgraphRequestCompressionConfig,
        accept_encoding: AcceptEncodingConfig,
        apq_enabled: bool,
    ) -> Self {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        header_map.insert(
            http::header::CONNECTION,
            HeaderValue::from_static("keep-alive"),
        );
        // Ask for the media types we can parse (see `VALID_GRAPHQL_CONTENT_TYPES`).
        // `header_map` is applied after header propagation, so a client's `Accept`
        // (e.g. `text/event-stream`) can't make the subgraph respond in a format we reject.
        header_map.insert(
            http::header::ACCEPT,
            HeaderValue::from_static(SUBGRAPH_ACCEPT_HEADER_VALUE),
        );

        // Advertise the algorithms we can decompress, regardless of whether outbound request
        // compression is enabled - unless the user disabled or emptied out the list.
        if let Some(value) = accept_encoding.header_value() {
            if let Ok(value) = HeaderValue::from_str(&value) {
                header_map.insert(http::header::ACCEPT_ENCODING, value);
            }
        }

        Self {
            subgraph_name,
            endpoint,
            http_client,
            header_map,
            semaphore,
            dedupe_enabled,
            in_flight_requests,
            telemetry_context,
            subgraph_buffer_capacity,
            compression,
            apq: SubgraphApq::new(apq_enabled),
        }
    }
}

pub struct SendRequestOpts<'a> {
    pub http_client: &'a HttpClientProvider,
    pub endpoint: &'a http::Uri,
    pub subgraph_name: &'a str,
    pub method: http::Method,
    pub body: Vec<u8>,
    pub headers: HeaderMap,
    pub timeout: Option<Duration>,
    pub telemetry_context: &'a Arc<TelemetryContext>,
    pub compression: TrafficShapingSubgraphRequestCompressionConfig,
}

/// The most we reserve up front from `content-length`. Past that the buffer grows as data
/// arrives, so a bogus header can't make us allocate a lot before reading anything.
const MAX_BODY_PREALLOCATION: usize = 16 * 1024 * 1024;

/// Reads the whole response body into one buffer. A body that arrives in one chunk is returned
/// as is. Otherwise every chunk is copied in and dropped right away, so hyper can reuse its read
/// buffer, instead of keeping all the chunks alive until the end and copying them then.
async fn read_body(mut body: hyper::body::Incoming) -> Result<Bytes, hyper::Error> {
    let Some(first) = next_chunk(&mut body).await? else {
        return Ok(Bytes::new());
    };
    let Some(second) = next_chunk(&mut body).await? else {
        return Ok(first);
    };

    let remaining = body.size_hint().lower() as usize;
    let mut buf = BytesMut::with_capacity(
        (first.len() + second.len()).saturating_add(remaining.min(MAX_BODY_PREALLOCATION)),
    );
    buf.extend_from_slice(&first);
    buf.extend_from_slice(&second);
    drop((first, second));
    while let Some(chunk) = next_chunk(&mut body).await? {
        buf.extend_from_slice(&chunk);
    }
    Ok(buf.freeze())
}

async fn next_chunk(body: &mut hyper::body::Incoming) -> Result<Option<Bytes>, hyper::Error> {
    while let Some(frame) = body.frame().await {
        // trailers aren't part of the body
        if let Ok(data) = frame?.into_data() {
            if !data.is_empty() {
                return Ok(Some(data));
            }
        }
    }
    Ok(None)
}

/// Awaits `fut`, giving up at `deadline` if there is one.
async fn until<F: std::future::Future>(
    deadline: Option<tokio::time::Instant>,
    fut: F,
) -> Result<F::Output, tokio::time::error::Elapsed> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, fut).await,
        None => Ok(fut.await),
    }
}

async fn send_request<'a>(
    opts: SendRequestOpts<'a>,
) -> Result<FetchedSubgraphResponse<'a>, SubgraphExecutorError> {
    let SendRequestOpts {
        http_client,
        endpoint,
        subgraph_name,
        method,
        body,
        mut headers,
        timeout,
        telemetry_context,
        compression,
    } = opts;

    let body = if compression.enabled {
        match compression::compress(&body, compression.algorithm) {
            Some(compressed) => {
                headers.insert(
                    http::header::CONTENT_ENCODING,
                    HeaderValue::from_static(compression.algorithm.token()),
                );
                compressed
            }
            // compression is best-effort: if it fails, send the original body uncompressed
            // rather than failing the whole subgraph request over it.
            None => body,
        }
    } else {
        body
    };

    let request_body_size = body.len() as u64;

    let mut req = hyper::Request::builder()
        .method(method)
        .uri(endpoint)
        .version(Version::HTTP_11)
        .body(Full::new(Bytes::from(body)))?;

    *req.headers_mut() = headers;

    debug!(target: targets::HTTP_CLIENT, endpoint = ?endpoint, subgraph = subgraph_name, "making http request");

    let http_request_span = HttpClientRequestSpan::from_request(&req);
    let mut http_request_capture = telemetry_context.metrics.http_client.capture_request(
        &req,
        request_body_size,
        Some(subgraph_name),
    );
    let transport_started_at = Instant::now();

    let response: Result<SubgraphHttpResponse, SubgraphExecutorError> = async {
        // TODO: let's decide at some point if the tracing headers
        //       should be part of the fingerprint or not.
        telemetry_context.inject_context_into_http_headers(req.headers_mut());

        // one deadline for the headers and the body, so a subgraph can't stall mid-body forever
        let deadline = timeout.map(|timeout| tokio::time::Instant::now() + timeout);
        let res = until(deadline, http_client.request(req)).await??;

        http_request_span.record_response(&res);
        http_request_capture.set_status_code(res.status().as_u16());

        debug!(
            target: targets::HTTP_CLIENT,
            endpoint = ?endpoint,
            subgraph = subgraph_name,
            status = res.status().as_u16(),
            "subgraph http request completed",
        );

        let (parts, body) = res.into_parts();

        let body = match until(deadline, read_body(body)).await? {
            Ok(body) => body,
            Err(err) => {
                return Err(SubgraphExecutorError::ResponseBodyReadFailure(
                    endpoint.to_string(),
                    err.to_string(),
                    parts.headers.into(),
                ));
            }
        };

        let body = match parts.headers.get(http::header::CONTENT_ENCODING) {
            // `identity` (RFC 9110 §8.4.1) and an empty token both mean "no encoding"
            Some(content_encoding) if !is_identity_or_empty_encoding(content_encoding) => {
                let algorithm = content_encoding
                    .to_str()
                    .ok()
                    .and_then(CompressionAlgorithm::from_token);
                let decompressed =
                    algorithm.and_then(|algorithm| compression::decompress(&body, algorithm));
                match decompressed {
                    Some(decompressed) => Bytes::from(decompressed),
                    None => {
                        return Err(SubgraphExecutorError::ResponseDecompressionFailure(
                            subgraph_name.to_string(),
                            content_encoding.to_str().unwrap_or("<invalid>").to_string(),
                            parts.headers.into(),
                        ));
                    }
                }
            }
            _ => body,
        };

        if body.is_empty() {
            return Err(SubgraphExecutorError::EmptyResponseBody(
                subgraph_name.to_string(),
                parts.headers.into(),
            ));
        }

        Ok(SubgraphHttpResponse {
            status: parts.status,
            body,
            headers: parts.headers.into(),
        })
    }
    .instrument(http_request_span.clone())
    .await;

    let transport_duration = transport_started_at.elapsed();

    match response {
        Ok(response) => Ok(FetchedSubgraphResponse {
            response,
            http_request_capture,
            transport_duration,
        }),
        Err(err) => {
            http_request_span.record_error(err.error_code());
            http_request_capture.finish_error(err.error_code(), transport_duration);
            Err(err)
        }
    }
}

pub enum DeduplicationHint {
    Deduped {
        fingerprint: u128,
        leader_id: u64,
        is_leader: bool,
    },
    NotDeduped,
}

impl HTTPSubgraphExecutor {
    /// One HTTP request. Gives the request back, without its headers, and whether a plugin
    /// answered instead of the subgraph or replaced its answer.
    async fn attempt<'s, 'a: 's, 'p: 's>(
        &'s self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        plugin_req_state: Option<&'p PluginRequestState<'p>>,
        apq: Apq<'_>,
    ) -> Result<
        (
            SubgraphResponse<'static>,
            SubgraphExecutionRequest<'s>,
            bool,
        ),
        SubgraphExecutorError,
    > {
        let mut execution_request: SubgraphExecutionRequest<'s> = execution_request;
        let mut body = build_request_body_with_apq(&execution_request, apq)?;

        self.header_map.iter().for_each(|(key, value)| {
            execution_request.headers.insert(key, value.clone());
        });

        let mut method = http::Method::POST;
        let mut deduplicate_request = !self.dedupe_enabled || !execution_request.dedupe;

        let mut on_end_callbacks = vec![];
        let mut response = None;

        if let Some(plugin_req_state) = plugin_req_state.as_ref() {
            let mut start_payload = OnSubgraphHttpRequestHookPayload {
                subgraph_name: &self.subgraph_name,
                endpoint: &self.endpoint,
                method,
                body,
                execution_request,
                deduplicate_request,
                context: &plugin_req_state.context,
                request_context: plugin_req_state
                    .request_context
                    .for_plugin::<hooks::OnSubgraphHttp>(),
            };
            for plugin in plugin_req_state.plugins.as_ref() {
                let result = plugin.on_subgraph_http_request(start_payload).await;
                start_payload = result.payload;
                match result.control_flow {
                    StartControlFlow::Proceed => { /* continue to next plugin */ }
                    StartControlFlow::EndWithResponse(early_response) => {
                        response = Some(early_response);
                        // Break so other plugins are not called
                        break;
                    }
                    StartControlFlow::OnEnd(callback) => {
                        on_end_callbacks.push(callback);
                    }
                }
            }
            // Give the ownership back to variables
            method = start_payload.method;
            execution_request = start_payload.execution_request;
            body = start_payload.body;
            deduplicate_request = start_payload.deduplicate_request;
        }

        let mut deduplication_hint = DeduplicationHint::NotDeduped;
        let mut http_request_capture = None;
        let mut answered_by_plugin = response.is_some();

        let mut response = match response {
            Some(resp) => resp,
            None => {
                let send_request_opts = SendRequestOpts {
                    http_client: &self.http_client,
                    endpoint: &self.endpoint,
                    subgraph_name: &self.subgraph_name,
                    method,
                    body,
                    headers: std::mem::take(&mut execution_request.headers),
                    timeout,
                    telemetry_context: &self.telemetry_context,
                    compression: self.compression,
                };

                if deduplicate_request {
                    // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
                    // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
                    let _permit = self.semaphore.acquire().await.unwrap();
                    let external_wait_guard = external_wait::enter();
                    let fetched_response = send_request(send_request_opts).await;
                    drop(external_wait_guard);
                    let fetched_response = fetched_response?;
                    http_request_capture = Some(HttpRequestTelemetryCapture {
                        capture: fetched_response.http_request_capture,
                        response_body_size: fetched_response.response.body.len() as u64,
                        transport_duration: fetched_response.transport_duration,
                    });
                    fetched_response.response
                } else {
                    let fingerprint = send_request_opts.fingerprint();

                    let inflight_span = HttpInflightRequestSpan::new(
                        &send_request_opts.method,
                        send_request_opts.endpoint,
                        &send_request_opts.headers,
                        &send_request_opts.body,
                    );

                    let result: Result<_, SubgraphExecutorError> = async {
                        let claim = self.in_flight_requests.claim(fingerprint);
                        let mut leader_http_request_capture = None;
                        let external_wait_guard = external_wait::enter();
                        let init_result = claim
                            .get_or_try_init(|| async {
                                let res = {
                                    // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
                                    // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
                                    let _permit = self.semaphore.acquire().await.unwrap();
                                    send_request(send_request_opts).await
                                };

                                res.map(|fetched_response| {
                                    leader_http_request_capture =
                                        Some(HttpRequestTelemetryCapture {
                                            capture: fetched_response.http_request_capture,
                                            response_body_size: fetched_response.response.body.len()
                                                as u64,
                                            transport_duration: fetched_response.transport_duration,
                                        });
                                    (fetched_response.response, unique_leader_fingerprint())
                                })
                            })
                            .await;
                        drop(external_wait_guard);
                        let (shared_response, role) = init_result?;

                        let (shared_response, leader_id) = shared_response.as_ref();
                        let shared_response = shared_response.clone();
                        let leader_id = *leader_id;

                        if role == InFlightRole::Leader {
                            inflight_span.record_as_leader(&leader_id);
                        } else {
                            inflight_span.record_as_joiner(&leader_id);
                        }

                        inflight_span
                            .record_response(&shared_response.body, &shared_response.status);

                        deduplication_hint = DeduplicationHint::Deduped {
                            fingerprint,
                            leader_id,
                            is_leader: role == InFlightRole::Leader,
                        };

                        Ok((shared_response, leader_http_request_capture))
                    }
                    .instrument(inflight_span.clone())
                    .await;

                    let (shared_response, leader_http_request_capture) = result?;
                    if let Some(capture) = leader_http_request_capture {
                        http_request_capture = Some(capture);
                    }

                    shared_response
                }
            }
        };

        if !on_end_callbacks.is_empty() {
            let plugin_state_ref = plugin_req_state
                .as_ref()
                .expect("plugin state not available, but on_end_callbacks are present");
            let mut end_payload = OnSubgraphHttpResponseHookPayload {
                context: &plugin_state_ref.context,
                request_context: plugin_state_ref
                    .request_context
                    .for_plugin::<hooks::OnSubgraphHttp>(),
                response,
                deduplication_hint,
            };
            for callback in on_end_callbacks {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    EndControlFlow::Proceed => { /* continue to next plugin */ }
                    EndControlFlow::EndWithResponse(early_response) => {
                        end_payload.response = early_response;
                        answered_by_plugin = true;
                        // Break so other plugins are not called
                        break;
                    }
                }
            }
            // Give the ownership back to variables
            response = end_payload.response;
        }

        let (is_content_type_valid, content_type) =
            validate_response_content_type(&response.headers);

        if !is_content_type_valid {
            return Err(SubgraphExecutorError::InvalidContentType(
                self.subgraph_name.clone(),
                content_type.unwrap_or("").to_string(),
                response.headers,
            ));
        }

        let response_result = response.deserialize_http_response(
            execution_request.custom_scalar_paths,
            execution_request.response_shape,
        );

        if let Some(mut http_request_capture) = http_request_capture {
            finish_capture_from_subgraph_result(
                &mut http_request_capture.capture,
                http_request_capture.response_body_size,
                http_request_capture.transport_duration,
                &response_result,
            );
        }

        response_result.map(|response| (response, execution_request, answered_by_plugin))
    }

    async fn send_hash_only<'s, 'a: 's, 'p: 's>(
        &'s self,
        mut execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        plugin_req_state: Option<&'p PluginRequestState<'p>>,
        hash: &str,
    ) -> Result<HashOnlyResult<'s>, SubgraphExecutorError> {
        // a retry starts from the request as it came in, hooks can change all of it.
        // The representations can be big, so the retry takes them from the first attempt
        let has_hooks = plugin_req_state.is_some_and(|state| !state.plugins.is_empty());
        let original = has_hooks.then(|| {
            let raw_variable_values = execution_request.raw_variable_values.take();
            let original = execution_request.clone();
            execution_request.raw_variable_values = raw_variable_values;
            original
        });
        let headers = if has_hooks {
            HeaderMap::new()
        } else {
            execution_request.headers.clone()
        };

        let is_query = execution_request.is_query;
        let (response, execution_request, answered_by_plugin) = self
            .attempt(
                execution_request,
                timeout,
                plugin_req_state,
                Apq::HashOnly(hash),
            )
            .await?;
        let apq_error = match apq_rejection(&response) {
            // a plugin's answer is final
            _ if answered_by_plugin => None,
            // only an APQ error proves a mutation didn't run
            Some(ApqError::Failed) if !is_query => None,
            apq_error => apq_error,
        };
        let Some(apq_error) = apq_error else {
            return Ok(HashOnlyResult::Done(response));
        };
        let retry = match original {
            Some(original) => SubgraphExecutionRequest {
                raw_variable_values: execution_request.raw_variable_values,
                ..original
            },
            None => SubgraphExecutionRequest {
                headers,
                ..execution_request
            },
        };
        Ok(HashOnlyResult::Rejected(retry, apq_error))
    }

    fn turn_apq_off(&self) {
        if self.apq.record_not_supported() {
            warn!(
                target: targets::HTTP_CLIENT,
                subgraph = self.subgraph_name,
                "subgraph doesn't support automatic persisted queries, turning them off for {}s",
                APQ_COOLDOWN.as_secs(),
            );
        }
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    fn executor_name(&self) -> &str {
        "http"
    }

    fn endpoint(&self) -> &http::Uri {
        &self.endpoint
    }

    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        plugin_req_state: Option<&'a PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        if !self.apq.should_send_hash() {
            return self
                .attempt(execution_request, timeout, plugin_req_state, Apq::Off)
                .await
                .map(|(response, ..)| response);
        }

        let started = Instant::now();
        let hash = query_hash(&execution_request);
        let (execution_request, apq_error) = match self
            .send_hash_only(execution_request, timeout, plugin_req_state, &hash)
            .await?
        {
            HashOnlyResult::Done(response) => return Ok(response),
            HashOnlyResult::Rejected(execution_request, apq_error) => {
                (execution_request, apq_error)
            }
        };

        if apq_error == ApqError::NotSupported {
            self.turn_apq_off();
        }
        let apq = match apq_error {
            ApqError::NotFound => Apq::QueryAndHash(&hash),
            ApqError::NotSupported | ApqError::Failed => Apq::Off,
        };

        // both attempts share one timeout
        let timeout = timeout.map(|timeout| timeout.saturating_sub(started.elapsed()));
        let (response, ..) = self
            .attempt(execution_request, timeout, plugin_req_state, apq)
            .await?;
        // the plain query worked where the hash didn't, so the subgraph ignores APQ
        if apq_error == ApqError::Failed && !response.data.is_null() {
            self.turn_apq_off();
        }
        Ok(response)
    }

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        connection_timeout: Option<Duration>,
    ) -> Result<
        BoxStream<'static, Result<SubgraphResponse<'static>, SubgraphExecutorError>>,
        SubgraphExecutorError,
    > {
        let custom_scalar_paths = execution_request.custom_scalar_paths.cloned();
        let buffer_capacity = self.subgraph_buffer_capacity;
        let body = build_request_body(&execution_request)?;

        let mut req = hyper::Request::builder()
            .method(http::Method::POST)
            .uri(&self.endpoint)
            .version(Version::HTTP_11)
            .body(Full::new(Bytes::from(body)))?;

        let mut headers = execution_request.headers;
        self.header_map.iter().for_each(|(key, value)| {
            headers.insert(key, value.clone());
        });

        // Prefer multipart over SSE for subscriptions
        // https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/multipart-protocol
        headers.insert(
            http::header::ACCEPT,
            HeaderValue::from_static(
                r#"multipart/mixed;subscriptionSpec="1.0", text/event-stream"#,
            ),
        );
        *req.headers_mut() = headers;

        debug!(
            target: targets::HTTP_CLIENT,
            subgraph = self.subgraph_name,
            endpoint = ?self.endpoint,
            "establishing subscription connection"
        );

        let res_fut = self.http_client.request(req);

        let res = if let Some(timeout_duration) = connection_timeout {
            tokio::time::timeout(timeout_duration, res_fut).await?
        } else {
            res_fut.await
        }?;

        debug!(
            target: targets::HTTP_CLIENT,
            subgraph = self.subgraph_name,
            endpoint = ?self.endpoint,
            status = res.status().as_u16(),
            "subscription connection established"
        );

        if !res.status().is_success() {
            return Err(SubgraphExecutorError::StreamStatusCodeNotOk(res.status()));
        }

        let (parts, body_stream) = res.into_parts();

        let content_type = parts
            .headers
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let is_multipart = content_type.starts_with("multipart/mixed");
        let is_sse = content_type.starts_with("text/event-stream");

        if !is_multipart && !is_sse {
            return Err(SubgraphExecutorError::UnsupportedContentTypeError(
                content_type.to_string(),
            ));
        }

        if is_multipart {
            debug!(
                target: targets::HTTP_CLIENT,
                subgraph = self.subgraph_name,
                "using multipart HTTP for subscription",
            );

            let boundary = multipart_subscribe::parse_boundary_from_header(content_type)
                .map_err(|e| SubgraphExecutorError::MultipartBoundaryParseFailure(e.to_string()))?;
            let stream = multipart_subscribe::parse_to_stream(
                boundary,
                body_stream,
                custom_scalar_paths.clone(),
            );

            let op_guard = self
                .telemetry_context
                .metrics
                .subscriptions
                .active_subgraph_operation(&self.subgraph_name);
            let conn_guard = self
                .telemetry_context
                .metrics
                .subscriptions
                .active_subgraph_connection(
                    &self.subgraph_name,
                    SubscriptionTransport::HttpMultipart,
                );

            let mapped = Box::pin(async_stream::stream! {
                let _op_guard = op_guard;
                let _conn_guard = conn_guard;

                trace!(target: targets::HTTP_CLIENT, "multipart subscription stream started");

                for await result in stream {
                    match result {
                        Ok(response) => {
                            trace!(target: targets::HTTP_CLIENT, response = ?response, "multipart subscription event received");

                            yield Ok(response);
                        }
                        Err(e) => {
                            yield Err(SubgraphExecutorError::MultipartStreamError(e.to_string()));
                            return;
                        }
                    }
                }
            });

            // buffer decouples the emitting subgraph from slow downstream consumers, dropping
            // messages under backpressure instead of throttling the subgraph
            Ok(subscription_buffer::buffered(
                mapped,
                buffer_capacity,
                self.telemetry_context.clone(),
                SubscriptionTransport::HttpMultipart,
                self.subgraph_name.clone(),
                self.endpoint.to_string(),
            ))
        } else {
            debug!(
                target: targets::HTTP_CLIENT,
                subgraph = self.subgraph_name,
                endpoint = ?self.endpoint,
                "using SSE for subscription connection to subgraph",
            );

            let stream = sse::parse_to_stream(body_stream, custom_scalar_paths.clone());

            let op_guard = self
                .telemetry_context
                .metrics
                .subscriptions
                .active_subgraph_operation(&self.subgraph_name);
            let conn_guard = self
                .telemetry_context
                .metrics
                .subscriptions
                .active_subgraph_connection(&self.subgraph_name, SubscriptionTransport::HttpSse);

            let mapped = Box::pin(async_stream::stream! {
                let _op_guard = op_guard;
                let _conn_guard = conn_guard;

                trace!(target: targets::HTTP_CLIENT, "SSE subscription stream started");
                for await result in stream {
                    match result {
                        Ok(response) => {
                            trace!(target: targets::HTTP_CLIENT, response = ?response, "SSE subscription event received");
                            yield Ok(response);
                        }
                        Err(e) => {
                            yield Err(SubgraphExecutorError::SseStreamError(e.to_string()));
                            return;
                        }
                    }
                }
            });

            // buffer decouples the emitting subgraph from slow downstream consumers, dropping
            // messages under backpressure instead of throttling the subgraph
            Ok(subscription_buffer::buffered(
                mapped,
                buffer_capacity,
                self.telemetry_context.clone(),
                SubscriptionTransport::HttpSse,
                self.subgraph_name.clone(),
                self.endpoint.to_string(),
            ))
        }
    }
}

fn finish_capture_from_subgraph_result(
    capture: &mut HttpClientRequestStateCapture<'_>,
    response_body_size: u64,
    transport_duration: Duration,
    response_result: &Result<SubgraphResponse<'_>, SubgraphExecutorError>,
) {
    let error_code = response_result.as_ref().err().map(|err| err.error_code());
    let graphql_response_status = if response_result.as_ref().is_ok_and(|response| {
        response
            .errors
            .as_ref()
            .is_none_or(|errors| errors.is_empty())
    }) {
        GraphQLResponseStatus::Ok
    } else {
        GraphQLResponseStatus::Error
    };

    capture.finish(
        response_body_size,
        transport_duration,
        graphql_response_status,
        error_code,
    );
}

#[derive(Default, Clone)]
pub struct SubgraphHttpResponse {
    pub status: StatusCode,
    pub headers: Arc<HeaderMap>,
    pub body: Bytes,
}

const VALID_GRAPHQL_CONTENT_TYPES: [&str; 2] =
    ["application/json", "application/graphql-response+json"];

/// `Accept` sent on regular subgraph fetches, preferring `application/graphql-response+json`
/// as recommended by the GraphQL over HTTP spec.
const SUBGRAPH_ACCEPT_HEADER_VALUE: &str =
    "application/graphql-response+json, application/json;q=0.9";

/// Whether a `Content-Encoding` value means "no encoding" - either the explicit `identity`
/// token (RFC 9110 §8.4.1) or an empty value - so the body can be used as-is without
/// decompression
#[inline]
fn is_identity_or_empty_encoding(value: &HeaderValue) -> bool {
    match value.to_str() {
        Ok(token) => {
            let token = token.trim();
            token.is_empty() || token.eq_ignore_ascii_case("identity")
        }
        Err(_) => false,
    }
}

#[inline]
fn is_valid_graphql_content_type(header_value: &str) -> bool {
    // Split by `;` to remove any charset or boundary parameters
    // and then take the first one.
    // We can't rely on simple string comparison because of additianal attributes (like charset),
    // and we can't rely on starts_with because there are valid values that start with `application/json` and can confuse our check.
    let essence = header_value.split(';').next().unwrap_or("").trim();

    VALID_GRAPHQL_CONTENT_TYPES
        .iter()
        .any(|valid| valid.eq_ignore_ascii_case(essence))
}

#[inline]
fn validate_response_content_type(headers: &HeaderMap) -> (bool, Option<&str>) {
    let content_type = headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());

    let Some(content_type) = content_type else {
        return (false, None);
    };

    if !is_valid_graphql_content_type(content_type) {
        return (false, Some(content_type));
    }

    (true, Some(content_type))
}

impl SubgraphHttpResponse {
    fn deserialize_http_response(
        self,
        custom_scalar_paths: Option<&CustomScalarPaths>,
        response_shape: Option<&ResponseShape>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        match response_shape {
            Some(shape) => {
                SubgraphResponse::deserialize_with_shape(self.body, shape, custom_scalar_paths)
            }
            None => SubgraphResponse::deserialize_from_bytes(self.body, custom_scalar_paths),
        }
        .map(|mut resp: SubgraphResponse| {
            resp.headers = Some(self.headers.clone());
            resp.status = Some(self.status);
            resp
        })
        .map_err(|e| match e {
            SubgraphExecutorError::ResponseDeserializationFailure(err, _) => {
                SubgraphExecutorError::ResponseDeserializationFailure(err, Some(self.headers))
            }
            other => other,
        })
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderMap;
    use serde_json::json;

    use super::build_request_body;
    use crate::executor::executors::common::SubgraphExecutionRequest;

    #[test]
    fn build_request_body_with_forwarded_operation_name_and_no_variables_is_valid_json() {
        let execution_request = SubgraphExecutionRequest {
            query: "query { me { id } }",
            document_name_write_pos: 5,
            dedupe: false,
            operation_name: Some("GetMe_accounts_0".to_string()),
            variables: None,
            headers: HeaderMap::new(),
            raw_variable_values: None,
            extensions: None,
            custom_scalar_paths: None,
            response_shape: None,
            connection_fingerprint: None,
            query_hash: None,
            is_query: false,
        };

        let body = build_request_body(&execution_request).expect("request body should serialize");
        let json_body: serde_json::Value =
            serde_json::from_slice(&body).expect("request body should be valid JSON");

        assert_eq!(
            json_body,
            json!({
                "query": "query GetMe_accounts_0 { me { id } }",
                "operationName": "GetMe_accounts_0",
            })
        );
    }

    #[test]
    fn apq_hash_covers_forwarded_operation_name() {
        use super::{build_request_body_with_apq, query_hash, Apq};

        let id = sonic_rs::json!("1");
        let execution_request = SubgraphExecutionRequest {
            query: "query($id:ID!){user(id:$id){id}}",
            document_name_write_pos: 5,
            dedupe: false,
            operation_name: Some("GetUser_accounts_0".to_string()),
            variables: Some([("id", &id)].into_iter().collect()),
            headers: HeaderMap::new(),
            raw_variable_values: None,
            extensions: None,
            custom_scalar_paths: None,
            response_shape: None,
            connection_fingerprint: None,
            query_hash: None,
            is_query: false,
        };
        let hash = query_hash(&execution_request);

        let body = build_request_body_with_apq(&execution_request, Apq::QueryAndHash(&hash))
            .expect("request body should serialize");
        let with_query: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let query = with_query["query"].as_str().unwrap();
        assert_eq!(query, "query GetUser_accounts_0($id:ID!){user(id:$id){id}}");
        assert_eq!(
            hash,
            "dc1e705408c9a80c7499e0a849917146cb42cb2eee65281ad68ea203b4cac8c1"
        );

        let body = build_request_body_with_apq(&execution_request, Apq::HashOnly(&hash))
            .expect("request body should serialize");
        let hash_only: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            hash_only,
            json!({
                "variables": { "id": "1" },
                "operationName": "GetUser_accounts_0",
                "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } },
            })
        );
    }

    #[test]
    fn apq_cooldown() {
        use super::{SubgraphApq, APQ_COOLDOWN};
        let off_until = 2_000 + APQ_COOLDOWN.as_millis() as u64;

        assert!(!SubgraphApq::new(false).should_send_hash());
        let apq = SubgraphApq::new(true);
        assert!(apq.should_send_hash());

        assert!(apq.record_not_supported_at(1_000));
        assert!(!apq.record_not_supported_at(2_000));
        assert!(!apq.record_not_supported_at(1_500));
        assert!(!apq.cooldown_over(off_until, off_until - 1));
        // renewed in the meantime
        assert!(!apq.cooldown_over(off_until - 1_000, off_until));
        assert!(apq.cooldown_over(off_until, off_until));
        assert!(apq.should_send_hash());
    }

    #[test]
    fn apq_rejection_only_when_nothing_ran() {
        use super::{apq_rejection, ApqError};
        use crate::executor::response::subgraph_response::SubgraphResponse;

        let rejection = |body: &'static str| {
            apq_rejection(&SubgraphResponse::deserialize_from_bytes(body.into(), None).unwrap())
        };

        assert_eq!(
            rejection(
                r#"{"errors":[{"message":"x","extensions":{"code":"PERSISTED_QUERY_NOT_FOUND"}}]}"#
            ),
            Some(ApqError::NotFound)
        );
        assert_eq!(
            rejection(r#"{"data":null,"errors":[{"message":"PersistedQueryNotSupported"}]}"#),
            Some(ApqError::NotSupported)
        );
        assert_eq!(
            rejection(r#"{"errors":[{"message":"Must provide query string."}]}"#),
            Some(ApqError::Failed)
        );
        assert_eq!(
            rejection(r#"{"data":{"a":1},"errors":[{"message":"PersistedQueryNotFound"}]}"#),
            None
        );
        assert_eq!(
            rejection(
                r#"{"data":null,"errors":[{"message":"PersistedQueryNotFound","path":["a"]}]}"#
            ),
            None
        );
    }
}
