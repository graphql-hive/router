use crate::cache_state::build_cache;
use crate::config::traffic_shaping::{
    TrafficShapingRouterDedupeHeadersConfig, TrafficShapingRouterDedupeHeadersKeyword,
};
use crate::config::HiveRouterConfig;
use crate::executor::coprocessor::{CoprocessorError, CoprocessorRuntime};
use crate::executor::execution::plan::FailedExecutionResult;
use crate::executor::executors::common::InboundRequestFingerprint;
use crate::executor::executors::inflight::{InFlightCleanupGuard, InFlightMap};
use crate::executor::extensions::{compile::compile_extensions_plan, plan::ExtensionsPlan};
use crate::executor::headers::sanitizer::is_never_join_header;
use crate::executor::plugin_trait::RouterPluginBoxed;
use crate::telemetry::logging::targets;
use crate::telemetry::metrics::catalog::values::SubscriptionEndReason;
use crate::telemetry::metrics::subscription_metrics::SubscriptionTransport;
use crate::telemetry::metrics::Metrics;
use crate::telemetry::TelemetryContext;
use crate::vrl::expressions::{BooleanOrProgram, ExpressionCompileError};
use futures::Stream;
use graphql_tools::validation::{rules::ValidationOptions, validate::ValidationPlan};
use http::StatusCode;
use moka::future::Cache;
use moka::Expiry;
use ntex::web;
use ntex::{http::HeaderMap, util::Bytes};
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{
    collections::HashSet,
    sync::{Arc, OnceLock},
};
use tracing::debug;

#[cfg(not(feature = "graphiql"))]
use crate::http_utils::laboratory::{
    build_laboratory_seed, render_laboratory_html, LaboratoryConfigError,
};
use crate::jwt::context::JwtTokenPayload;
use crate::jwt::JwtAuthRuntime;
use crate::pipeline::active_subscriptions::{ActiveSubscriptions, SubscriptionEvent};
use crate::pipeline::cors::{CORSConfigError, Cors};
use crate::pipeline::error::{ClientPipelineError, PipelineError};
use crate::pipeline::header::{ResponseMode, StreamContentType};
use crate::pipeline::introspection_policy::compile_introspection_policy;
use crate::pipeline::long_lived_client_limit::LongLivedClientGuard;
use crate::pipeline::multipart_subscribe::{
    self, APOLLO_MULTIPART_HTTP_CONTENT_TYPE, INCREMENTAL_DELIVERY_CONTENT_TYPE,
};
use crate::pipeline::parser::ParseCacheEntry;
use crate::pipeline::sse;
use crate::storage::StorageManager;

pub type JwtClaimsCache = Cache<String, Arc<JwtTokenPayload>>;
pub type RouterInflightRequestsMap = InFlightMap<InboundRequestFingerprint, SharedRouterResponse>;

#[derive(Clone)]
pub enum RouterRequestDedupeHeaderPolicy {
    All,
    None,
    Include(HashSet<String>),
}

impl RouterRequestDedupeHeaderPolicy {
    #[inline]
    pub fn should_include(&self, header_name: &str) -> bool {
        match self {
            Self::All => true,
            Self::None => false,
            Self::Include(allowed_headers) => allowed_headers.contains(header_name),
        }
    }
}

impl From<&TrafficShapingRouterDedupeHeadersConfig> for RouterRequestDedupeHeaderPolicy {
    fn from(headers: &TrafficShapingRouterDedupeHeadersConfig) -> Self {
        match headers {
            TrafficShapingRouterDedupeHeadersConfig::Keyword(
                TrafficShapingRouterDedupeHeadersKeyword::All,
            ) => Self::All,
            TrafficShapingRouterDedupeHeadersConfig::Keyword(
                TrafficShapingRouterDedupeHeadersKeyword::None,
            ) => Self::None,
            TrafficShapingRouterDedupeHeadersConfig::Include { include } => {
                if include.is_empty() {
                    return Self::None;
                }

                let mut dedupe_headers = HashSet::with_capacity(include.len());
                for header in include {
                    dedupe_headers.insert(header.get_header_ref().as_str().to_owned());
                }

                Self::Include(dedupe_headers)
            }
        }
    }
}

pub type SharedRouterResponseGuard =
    InFlightCleanupGuard<InboundRequestFingerprint, SharedRouterResponse>;

#[derive(Clone)]
pub enum SharedRouterResponse {
    Single(SharedRouterSingleResponse),
    Stream(SharedRouterStreamResponse),
    Parts(SharedRouterPartsResponse),
}

impl SharedRouterResponse {
    pub fn error_count(&self) -> usize {
        match self {
            SharedRouterResponse::Single(resp) => resp.error_count,
            SharedRouterResponse::Stream(resp) => resp.error_count,
            SharedRouterResponse::Parts(resp) => {
                resp.error_count.load(std::sync::atomic::Ordering::Relaxed)
            }
        }
    }
    pub fn into_response(
        self,
        response_mode: &ResponseMode,
        metrics: &Arc<Metrics>,
        long_lived_client_guard: Option<LongLivedClientGuard>,
    ) -> Result<web::HttpResponse, PipelineError> {
        match self {
            // dropping the guard here would be a leak for streams, but singles
            // are not long-lived clients so releasing the slot right away is correct
            SharedRouterResponse::Single(single) => match response_mode {
                // a client that only takes streams, like `@defer(if: false)` from a client
                // that only accepts `multipart/mixed`, gets the response as the only part
                ResponseMode::StreamOnly(stream_content_type) => Ok(parts_response(
                    single.status,
                    &single.headers,
                    Some(stream_content_type.defer_content_type(false)),
                    futures::stream::once(futures::future::ready(single.body.to_vec())),
                    stream_content_type,
                )),
                _ => Ok(single.into()),
            },
            SharedRouterResponse::Stream(stream) => {
                let stream_content_type = response_mode
                    .stream_content_type()
                    .ok_or(ClientPipelineError::SubscriptionsTransportNotSupported)?;
                Ok(stream.into_response(stream_content_type, metrics, long_lived_client_guard))
            }
            SharedRouterResponse::Parts(parts) => {
                let stream_content_type = response_mode
                    .stream_content_type()
                    .ok_or(ClientPipelineError::DeferTransportNotSupported)?;
                Ok(parts.into_response(stream_content_type))
            }
        }
    }
}

/// An incremental (`@defer`) response. The primary part is ready, the rest are still coming:
/// deferred fetches run as the stream is read. The content type is in `headers`.
/// `@defer` requests aren't deduplicated, so the stream is only ever taken once.
#[derive(Clone)]
pub struct SharedRouterPartsResponse {
    /// The status of the response head, the primary part's, like a regular response's.
    pub status: StatusCode,
    pub primary: Bytes,
    pub rest: Arc<std::sync::Mutex<Option<futures::stream::BoxStream<'static, Vec<u8>>>>>,
    pub headers: Arc<HeaderMap>,
    /// The errors of the parts that went out so far, all of them once the stream ends.
    pub error_count: Arc<std::sync::atomic::AtomicUsize>,
}

impl SharedRouterPartsResponse {
    /// Runs `report` with the error count of every part once the last part went out, or with
    /// what went out when the client goes away first.
    pub fn on_end(
        self,
        report: impl FnOnce(usize) -> futures::future::BoxFuture<'static, ()> + Send + 'static,
    ) -> Self {
        /// Spawns the report when the stream is dropped before its end.
        struct Guard<F: FnOnce(usize) -> futures::future::BoxFuture<'static, ()>> {
            report: Option<F>,
            error_count: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl<F: FnOnce(usize) -> futures::future::BoxFuture<'static, ()>> Drop for Guard<F> {
            fn drop(&mut self) {
                if let Some(report) = self.report.take() {
                    let error_count = self.error_count.load(std::sync::atomic::Ordering::Relaxed);
                    ntex::rt::spawn(report(error_count));
                }
            }
        }

        let mut rest = self.take_rest();
        let mut guard = Guard {
            report: Some(report),
            error_count: self.error_count.clone(),
        };
        let stream = async_stream::stream! {
            use futures::StreamExt;
            while let Some(part) = rest.next().await {
                yield part;
            }
            // run here, the body is polled in the request's scope
            if let Some(report) = guard.report.take() {
                report(guard.error_count.load(std::sync::atomic::Ordering::Relaxed)).await;
            }
        };
        *self.rest_slot() = Some(Box::pin(stream));
        self
    }

    /// Where the parts after the primary one wait to be taken. A poisoned lock still holds the
    /// stream, nothing else touches it while the lock is held.
    fn rest_slot(
        &self,
    ) -> std::sync::MutexGuard<'_, Option<futures::stream::BoxStream<'static, Vec<u8>>>> {
        self.rest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The parts after the primary one. They're taken once, `@defer` responses aren't
    /// deduplicated, so a second reader is a bug and gets none.
    pub fn take_rest(&self) -> futures::stream::BoxStream<'static, Vec<u8>> {
        self.rest_slot().take().unwrap_or_else(|| {
            tracing::error!(
                target: crate::telemetry::logging::targets::GRAPHQL_EXECUTION,
                "the deferred parts of a response were already taken"
            );
            Box::pin(futures::stream::empty())
        })
    }

    pub fn into_response(self, stream_content_type: &StreamContentType) -> web::HttpResponse {
        use futures::StreamExt;
        let rest = self.take_rest();
        let parts =
            futures::stream::once(futures::future::ready(self.primary.to_vec())).chain(rest);
        parts_response(self.status, &self.headers, None, parts, stream_content_type)
    }
}

/// A response that sends `parts` as they come, as SSE events or multipart parts. `content_type`
/// replaces the one in `headers`.
fn parts_response(
    status: StatusCode,
    headers: &HeaderMap,
    content_type: Option<&'static str>,
    parts: impl Stream<Item = Vec<u8>> + Send + Unpin + 'static,
    stream_content_type: &StreamContentType,
) -> web::HttpResponse {
    let body: std::pin::Pin<
        Box<dyn Stream<Item = Result<ntex::util::Bytes, std::io::Error>> + Send>,
    > = match stream_content_type {
        StreamContentType::SSE => Box::pin(sse::create_stream(parts, sse::HEARTBEAT_INTERVAL)),
        _ => Box::pin(multipart_subscribe::create_incremental_delivery_stream(
            parts,
        )),
    };

    let mut response = web::HttpResponse::Ok();
    response.status(status);
    for (header_name, header_value) in headers.iter() {
        if is_never_join_header(header_name) {
            response.header(header_name, header_value);
        } else {
            response.set_header(header_name, header_value);
        }
    }
    if let Some(content_type) = content_type {
        response.set_header(ntex::http::header::CONTENT_TYPE, content_type);
    }
    // tells proxies like nginx not to hold the parts back
    response.set_header("x-accel-buffering", "no");
    response.streaming(body)
}

#[derive(Clone)]
pub struct SharedRouterSingleResponse {
    pub body: Bytes,
    pub headers: Arc<HeaderMap>,
    pub status: StatusCode,
    pub error_count: usize,
}

impl From<SharedRouterSingleResponse> for web::HttpResponse {
    fn from(shared_response: SharedRouterSingleResponse) -> Self {
        let mut response = web::HttpResponse::Ok();
        response.status(shared_response.status);

        for (header_name, header_value) in shared_response.headers.iter() {
            if is_never_join_header(header_name) {
                response.header(header_name, header_value);
            } else {
                response.set_header(header_name, header_value);
            }
        }

        response.body(shared_response.body)
    }
}

// status is always 200 for streaming responses, errors are sent through the stream.
// stream content type is not included because we can deduplicate subscriptions across
// different content types, the response format is decided when converting to response
pub struct SharedRouterStreamResponse {
    pub body: tokio::sync::broadcast::Sender<SubscriptionEvent>,
    pub headers: Arc<HeaderMap>,
    pub error_count: usize,
    // the leader gets the receiver that was subscribed before the pump was spawned, so
    // there is no window where the channel has zero receivers and events can be lost.
    // joiners get None and subscribe via body.subscribe() when consumed.
    pub receiver: Option<tokio::sync::broadcast::Receiver<SubscriptionEvent>>,
}

impl Clone for SharedRouterStreamResponse {
    fn clone(&self) -> Self {
        Self {
            body: self.body.clone(),
            headers: self.headers.clone(),
            error_count: self.error_count,
            receiver: None,
        }
    }
}

impl SharedRouterStreamResponse {
    pub fn into_response(
        self,
        stream_content_type: &StreamContentType,
        metrics: &Arc<Metrics>,
        long_lived_client_guard: Option<LongLivedClientGuard>,
    ) -> web::HttpResponse {
        // leader already has a pre-subscribed receiver to avoid missing
        // any potential events emitted. joiners, on the other hand, subscribe
        let mut receiver = self.receiver.unwrap_or_else(|| self.body.subscribe());

        let transport = match stream_content_type {
            StreamContentType::IncrementalDelivery(_) | StreamContentType::ApolloMultipartHTTP => {
                SubscriptionTransport::HttpMultipart
            }
            StreamContentType::SSE => SubscriptionTransport::HttpSse,
        };
        // each consumer (leader or joiner) gets its own guards, created here so dedup joiners
        // are counted too - dropped when the client stream below ends
        let mut client_op_guard = metrics.subscriptions.active_client_operation(transport);
        let client_conn_guard = metrics.subscriptions.active_client_connection(transport);
        let metrics = metrics.clone();

        let stream = Box::pin(async_stream::stream! {
            let _client_conn_guard = client_conn_guard;
            // holds the long-lived client slot until this client's stream ends
            let _long_lived_client_guard = long_lived_client_guard;
            loop {
                match receiver.recv().await {
                    Ok(SubscriptionEvent::Raw(data)) => {
                        yield data.to_vec();
                        metrics.subscriptions.record_client_sent(transport);
                    }
                    Ok(SubscriptionEvent::Error(errors)) => {
                        client_op_guard.set_end_reason(SubscriptionEndReason::Error);
                        yield FailedExecutionResult { errors }.serialize();
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // NOTE: not warn to avoid log spam when receiver starts lagging.
                        // users should rely on the lagged_messages metric to detect slow
                        // consumers and tune accordingly
                        debug!(target: targets::SUBSCRIPTIONS, lagged = n, "Broadcast receiver lagged, dropping message");
                        metrics.subscriptions.record_client_lag(transport, n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        client_op_guard.set_end_reason(SubscriptionEndReason::Completed);
                        break;
                    }
                }
            }
            let _client_op_guard = client_op_guard;
        });

        let content_type_header = match stream_content_type {
            StreamContentType::IncrementalDelivery(_) => {
                http::HeaderValue::from_static(INCREMENTAL_DELIVERY_CONTENT_TYPE)
            }
            StreamContentType::SSE => http::HeaderValue::from_static("text/event-stream"),
            StreamContentType::ApolloMultipartHTTP => {
                http::HeaderValue::from_static(APOLLO_MULTIPART_HTTP_CONTENT_TYPE)
            }
        };

        let body: std::pin::Pin<
            Box<dyn Stream<Item = Result<ntex::util::Bytes, std::io::Error>> + Send>,
        > = match stream_content_type {
            StreamContentType::IncrementalDelivery(_) => Box::pin(
                multipart_subscribe::create_incremental_delivery_stream(stream),
            ),
            StreamContentType::SSE => Box::pin(sse::create_stream(stream, sse::HEARTBEAT_INTERVAL)),
            StreamContentType::ApolloMultipartHTTP => {
                Box::pin(multipart_subscribe::create_apollo_multipart_http_stream(
                    stream,
                    sse::HEARTBEAT_INTERVAL,
                ))
            }
        };

        let mut response = web::HttpResponse::Ok();

        for (header_name, header_value) in self.headers.iter() {
            if is_never_join_header(header_name) {
                response.header(header_name, header_value);
            } else {
                response.set_header(header_name, header_value);
            }
        }

        // we set content type after so that we can override the shared header
        response.content_type(content_type_header);

        response.streaming(body)
    }
}

/// Default TTL for JWT claims cache entries (5 seconds)
const DEFAULT_JWT_CACHE_TTL_SECS: u64 = 5;

struct JwtClaimsExpiry;

impl Expiry<String, Arc<JwtTokenPayload>> for JwtClaimsExpiry {
    fn expire_after_create(
        &self,
        _key: &String,
        value: &Arc<JwtTokenPayload>,
        _created_at: std::time::Instant,
    ) -> Option<Duration> {
        const DEFAULT_TTL: Duration = Duration::from_secs(DEFAULT_JWT_CACHE_TTL_SECS);

        // if token has no exp claim, use default TTL (avoids syscall)
        let exp = match value.claims.exp {
            Some(e) => e,
            None => return Some(DEFAULT_TTL),
        };

        let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs(),
            Err(_) => return Some(DEFAULT_TTL), // Clock error: fall back to default
        };

        // If token is already expired, return zero TTL to remove it immediately
        if exp <= now {
            return Some(Duration::ZERO);
        }

        // Calculate time until token expiration
        let time_until_exp = Duration::from_secs(exp - now);

        // Return the minimum of default TTL and time until expiration.
        // Short-lived tokens (exp < 5s) are evicted when they expire
        // Long-lived tokens still respect the 5s cache limit.
        Some(DEFAULT_TTL.min(time_until_exp))
    }
}
pub struct RouterSharedState {
    pub validation_plan: Arc<ValidationPlan>,
    pub validation_options: ValidationOptions,
    pub parse_cache: Cache<u64, ParseCacheEntry>,
    pub router_config: &'static HiveRouterConfig,
    pub extensions_plan: Arc<ExtensionsPlan>,
    pub cors_runtime: Option<Cors>,
    /// Cache for validated JWT claims to avoid re-parsing on every request.
    /// The cache key is the raw JWT token string.
    /// Stores the parsed claims payload for 5s,
    /// but no longer than `exp` date.
    pub jwt_claims_cache: JwtClaimsCache,
    pub jwt_auth_runtime: Option<JwtAuthRuntime>,
    pub introspection_policy: BooleanOrProgram,
    pub telemetry_context: Arc<TelemetryContext>,
    pub coprocessor: Option<CoprocessorRuntime>,
    pub plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
    pub in_flight_requests: RouterInflightRequestsMap,
    pub in_flight_requests_header_policy: RouterRequestDedupeHeaderPolicy,
    /// Tracks the number of active long-lived clients (websockets + http streams)
    pub long_lived_client_count: Arc<AtomicUsize>,
    /// Tracks all active subscriptions from clients to the router.
    pub active_subscriptions: ActiveSubscriptions,
    /// The storage manager for the router.
    pub storage_manager: Arc<StorageManager>,
    laboratory_html: OnceLock<Bytes>,
}

impl RouterSharedState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        router_config: &'static HiveRouterConfig,
        jwt_auth_runtime: Option<JwtAuthRuntime>,
        validation_plan: ValidationPlan,
        validation_options: ValidationOptions,
        telemetry_context: Arc<TelemetryContext>,
        plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
        active_subscriptions: ActiveSubscriptions,
        storage_manager: Arc<StorageManager>,
    ) -> Result<Self, SharedStateError> {
        let parse_cache = build_cache(&router_config.cache.router.parsing);
        let coprocessor = router_config
            .coprocessor
            .as_ref()
            .map(|coprocessor_config| {
                CoprocessorRuntime::from_config(
                    coprocessor_config,
                    telemetry_context.clone(),
                    router_config.limits.max_request_body_size.to_bytes() as usize,
                    router_config.traffic_shaping.router.body_read_timeout,
                )
                .map_err(Box::new)
            })
            .transpose()?;
        check_laboratory_config(&router_config.laboratory)?;

        Ok(Self {
            validation_plan: Arc::new(validation_plan),
            validation_options,
            extensions_plan: Arc::new(compile_extensions_plan(&router_config.response_extensions)),
            parse_cache,
            cors_runtime: Cors::from_config(&router_config.cors).map_err(Box::new)?,
            jwt_claims_cache: Cache::builder()
                // High capacity due to potentially high token diversity.
                // Capping prevents unbounded memory usage.
                .max_capacity(10_000)
                .expire_after(JwtClaimsExpiry)
                .build(),
            router_config,
            jwt_auth_runtime,
            introspection_policy: compile_introspection_policy(&router_config.introspection)
                .map_err(Box::new)?,
            telemetry_context,
            coprocessor,
            plugins,
            in_flight_requests: InFlightMap::default(),
            in_flight_requests_header_policy: (&router_config
                .traffic_shaping
                .router
                .dedupe
                .headers)
                .into(),
            long_lived_client_count: Arc::new(AtomicUsize::new(0)),
            active_subscriptions,
            storage_manager,
            laboratory_html: OnceLock::new(),
        })
    }
}

impl RouterSharedState {
    pub fn laboratory_page(&self) -> Bytes {
        self.laboratory_html
            .get_or_init(|| render_laboratory_page(&self.router_config.laboratory))
            .clone()
    }
}

/// Fails startup on a bad config, even though the page itself is rendered later.
#[cfg(not(feature = "graphiql"))]
fn check_laboratory_config(
    config: &crate::config::laboratory::LaboratoryConfig,
) -> Result<(), SharedStateError> {
    build_laboratory_seed(config).map_err(Box::new)?;
    Ok(())
}

#[cfg(not(feature = "graphiql"))]
fn render_laboratory_page(config: &crate::config::laboratory::LaboratoryConfig) -> Bytes {
    render_laboratory_html(crate::LABORATORY_HTML, config)
        .expect("the laboratory config is checked at startup")
}

/// The `graphiql` feature swaps the Laboratory for a static GraphiQL page, which has nowhere to
/// put the seed values.
#[cfg(feature = "graphiql")]
fn check_laboratory_config(
    config: &crate::config::laboratory::LaboratoryConfig,
) -> Result<(), SharedStateError> {
    if !config.operations.is_empty()
        || !config.collections.is_empty()
        || !config.global_headers.is_empty()
    {
        tracing::warn!(
          target: targets::CORE,
          "'laboratory.operations', 'laboratory.collections' and 'laboratory.global_headers' are ignored because this router was built with the 'graphiql' feature."
        );
    }

    Ok(())
}

#[cfg(feature = "graphiql")]
fn render_laboratory_page(_config: &crate::config::laboratory::LaboratoryConfig) -> Bytes {
    Bytes::from_static(crate::LABORATORY_HTML.as_bytes())
}

#[derive(thiserror::Error, Debug)]
pub enum SharedStateError {
    #[error("invalid regex in CORS config: {0}")]
    CORSConfig(#[from] Box<CORSConfigError>),
    #[error("invalid introspection config: {0}")]
    IntrospectionPolicyCompile(#[from] Box<ExpressionCompileError>),
    #[error("invalid coprocessor config: {0}")]
    CoprocessorRuntime(#[from] Box<CoprocessorError>),
    #[cfg(not(feature = "graphiql"))]
    #[error("invalid laboratory config: {0}")]
    LaboratoryConfig(#[from] Box<LaboratoryConfigError>),
}

#[cfg(test)]
mod tests {
    use super::RouterRequestDedupeHeaderPolicy;
    use crate::config::traffic_shaping::{
        TrafficShapingRouterDedupeHeadersConfig, TrafficShapingRouterDedupeHeadersKeyword,
    };

    #[test]
    fn should_map_header_variants_to_policy() {
        let all = TrafficShapingRouterDedupeHeadersConfig::Keyword(
            TrafficShapingRouterDedupeHeadersKeyword::All,
        );
        assert!(matches!(
            RouterRequestDedupeHeaderPolicy::from(&all),
            RouterRequestDedupeHeaderPolicy::All
        ));

        let none = TrafficShapingRouterDedupeHeadersConfig::Keyword(
            TrafficShapingRouterDedupeHeadersKeyword::None,
        );
        assert!(matches!(
            RouterRequestDedupeHeaderPolicy::from(&none),
            RouterRequestDedupeHeaderPolicy::None
        ));

        let include = TrafficShapingRouterDedupeHeadersConfig::Include {
            include: vec!["Authorization".into()],
        };
        let include_policy = RouterRequestDedupeHeaderPolicy::from(&include);
        assert!(matches!(
            include_policy,
            RouterRequestDedupeHeaderPolicy::Include(_)
        ));
        assert!(include_policy.should_include("authorization"));
    }
}
