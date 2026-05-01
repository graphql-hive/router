use futures::Stream;
use graphql_tools::validation::validate::ValidationPlan;
use hive_console_sdk::agent::usage_agent::{AgentError, UsageAgent};
use hive_router_config::traffic_shaping::{
    TrafficShapingRouterDedupeHeadersConfig, TrafficShapingRouterDedupeHeadersKeyword,
};
use hive_router_config::HiveRouterConfig;
use hive_router_internal::expressions::values::boolean::BooleanOrProgram;
use hive_router_internal::expressions::ExpressionCompileError;
use hive_router_internal::inflight::{InFlightCleanupGuard, InFlightMap};
use hive_router_internal::telemetry::TelemetryContext;
use hive_router_plan_executor::coprocessor::{CoprocessorError, CoprocessorRuntime};
use hive_router_plan_executor::execution::plan::FailedExecutionResult;
use hive_router_plan_executor::headers::{
    compile::compile_headers_plan, errors::HeaderRuleCompileError, plan::HeaderRulesPlan,
};
use hive_router_plan_executor::plugin_trait::RouterPluginBoxed;
use http::StatusCode;
use moka::future::Cache;
use moka::Expiry;
use ntex::web;
use ntex::{http::HeaderMap, util::Bytes};
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{collections::HashSet, sync::Arc};
use tracing::trace;

use crate::cache_state::CacheState;
use crate::jwt::context::JwtTokenPayload;
use crate::jwt::JwtAuthRuntime;
use crate::pipeline::active_subscriptions::{ActiveSubscriptions, SubscriptionEvent};
use crate::pipeline::cors::{CORSConfigError, Cors};
use crate::pipeline::error::PipelineError;
use crate::pipeline::header::{ResponseMode, StreamContentType};
use crate::pipeline::introspection_policy::compile_introspection_policy;
use crate::pipeline::multipart_subscribe::{
    self, APOLLO_MULTIPART_HTTP_CONTENT_TYPE, INCREMENTAL_DELIVERY_CONTENT_TYPE,
};
use crate::pipeline::parser::ParseCacheEntry;
use crate::pipeline::persisted_documents::resolve::PersistedDocumentResolverError;
use crate::pipeline::persisted_documents::PersistedDocumentsRuntime;
use crate::pipeline::progressive_override::{OverrideLabelsCompileError, OverrideLabelsEvaluator};
use crate::pipeline::sse;

pub type JwtClaimsCache = Cache<String, Arc<JwtTokenPayload>>;
pub type RouterInflightRequestsMap = InFlightMap<u64, SharedRouterResponse>;

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

pub type SharedRouterResponseGuard = InFlightCleanupGuard<u64, SharedRouterResponse>;

#[derive(Clone)]
pub enum SharedRouterResponse {
    Single(SharedRouterSingleResponse),
    Stream(SharedRouterStreamResponse),
}

impl SharedRouterResponse {
    pub fn error_count(&self) -> usize {
        match self {
            SharedRouterResponse::Single(resp) => resp.error_count,
            SharedRouterResponse::Stream(resp) => resp.error_count,
        }
    }
    pub fn into_response(
        self,
        response_mode: &ResponseMode,
    ) -> Result<web::HttpResponse, PipelineError> {
        match self {
            SharedRouterResponse::Single(single) => Ok(single.into()),
            SharedRouterResponse::Stream(stream) => {
                let stream_content_type = response_mode
                    .stream_content_type()
                    .ok_or(PipelineError::SubscriptionsTransportNotSupported)?;
                Ok(stream.into_response(stream_content_type))
            }
        }
    }
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
            response.set_header(header_name, header_value);
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
    pub fn into_response(self, stream_content_type: &StreamContentType) -> web::HttpResponse {
        // leader already has a pre-subscribed receiver to avoid missing
        // any potential events emitted. joiners, on the other hand, subscribe
        let mut receiver = self.receiver.unwrap_or_else(|| self.body.subscribe());

        let stream = Box::pin(async_stream::stream! {
            loop {
                match receiver.recv().await {
                    Ok(SubscriptionEvent::Raw(data)) => {
                        yield data.to_vec();
                    }
                    Ok(SubscriptionEvent::Error(errors)) => {
                        yield FailedExecutionResult { errors }.serialize();
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        trace!(lagged = n, "broadcast receiver lagged, skipping missed messages");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        });

        let content_type_header = match stream_content_type {
            StreamContentType::IncrementalDelivery => {
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
            StreamContentType::IncrementalDelivery => Box::pin(
                multipart_subscribe::create_incremental_delivery_stream(stream),
            ),
            StreamContentType::SSE => Box::pin(sse::create_stream(
                stream,
                std::time::Duration::from_secs(10),
            )),
            StreamContentType::ApolloMultipartHTTP => {
                Box::pin(multipart_subscribe::create_apollo_multipart_http_stream(
                    stream,
                    std::time::Duration::from_secs(10),
                ))
            }
        };

        let mut response = web::HttpResponse::Ok();

        for (header_name, header_value) in self.headers.iter() {
            response.set_header(header_name, header_value);
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
    pub parse_cache: Cache<u64, ParseCacheEntry>,
    pub persisted_documents_runtime: PersistedDocumentsRuntime,
    pub router_config: Arc<HiveRouterConfig>,
    pub headers_plan: Arc<HeaderRulesPlan>,
    pub override_labels_evaluator: OverrideLabelsEvaluator,
    pub cors_runtime: Option<Cors>,
    /// Cache for validated JWT claims to avoid re-parsing on every request.
    /// The cache key is the raw JWT token string.
    /// Stores the parsed claims payload for 5s,
    /// but no longer than `exp` date.
    pub jwt_claims_cache: JwtClaimsCache,
    pub jwt_auth_runtime: Option<JwtAuthRuntime>,
    pub hive_usage_agent: Option<UsageAgent>,
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
}

impl RouterSharedState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        router_config: Arc<HiveRouterConfig>,
        persisted_documents_runtime: PersistedDocumentsRuntime,
        jwt_auth_runtime: Option<JwtAuthRuntime>,
        hive_usage_agent: Option<UsageAgent>,
        validation_plan: ValidationPlan,
        telemetry_context: Arc<TelemetryContext>,
        plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
        cache_state: Arc<CacheState>,
        active_subscriptions: ActiveSubscriptions,
    ) -> Result<Self, SharedStateError> {
        let parse_cache = cache_state.parse_cache.clone();
        let coprocessor = router_config
            .coprocessor
            .as_ref()
            .map(|coprocessor_config| {
                CoprocessorRuntime::from_config(
                    coprocessor_config,
                    telemetry_context.clone(),
                    router_config.limits.max_request_body_size.to_bytes() as usize,
                )
                .map_err(Box::new)
            })
            .transpose()?;

        Ok(Self {
            validation_plan: Arc::new(validation_plan),
            headers_plan: Arc::new(compile_headers_plan(&router_config.headers).map_err(Box::new)?),
            parse_cache,
            persisted_documents_runtime,
            cors_runtime: Cors::from_config(&router_config.cors).map_err(Box::new)?,
            jwt_claims_cache: Cache::builder()
                // High capacity due to potentially high token diversity.
                // Capping prevents unbounded memory usage.
                .max_capacity(10_000)
                .expire_after(JwtClaimsExpiry)
                .build(),
            router_config: router_config.clone(),
            override_labels_evaluator: OverrideLabelsEvaluator::from_config(
                &router_config.override_labels,
            )
            .map_err(Box::new)?,
            jwt_auth_runtime,
            hive_usage_agent,
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
        })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SharedStateError {
    #[error("invalid headers config: {0}")]
    HeaderRuleCompile(#[from] Box<HeaderRuleCompileError>),
    #[error("invalid regex in CORS config: {0}")]
    CORSConfig(#[from] Box<CORSConfigError>),
    #[error("invalid override labels config: {0}")]
    OverrideLabelsCompile(#[from] Box<OverrideLabelsCompileError>),
    #[error("error creating hive usage agent: {0}")]
    UsageAgent(#[from] Box<AgentError>),
    #[error("invalid persisted documents config: {0}")]
    PersistedDocuments(#[from] Box<PersistedDocumentResolverError>),
    #[error("invalid introspection config: {0}")]
    IntrospectionPolicyCompile(#[from] Box<ExpressionCompileError>),
    #[error("invalid coprocessor config: {0}")]
    CoprocessorRuntime(#[from] Box<CoprocessorError>),
}

#[cfg(test)]
mod tests {
    use super::RouterRequestDedupeHeaderPolicy;
    use hive_router_config::traffic_shaping::{
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
