use graphql_tools::validation::validate::ValidationPlan;
use hive_console_sdk::agent::usage_agent::{AgentError, UsageAgent};
use hive_router_config::traffic_shaping::{
    TrafficShapingRouterDedupeHeadersConfig, TrafficShapingRouterDedupeHeadersKeyword,
};
use hive_router_config::HiveRouterConfig;
use hive_router_internal::expressions::values::boolean::BooleanOrProgram;
use hive_router_internal::expressions::ExpressionCompileError;
use hive_router_internal::inflight::InFlightMap;
use hive_router_internal::telemetry::TelemetryContext;
use hive_router_plan_executor::headers::{
    compile::compile_headers_plan, errors::HeaderRuleCompileError, plan::HeaderRulesPlan,
};
use hive_router_plan_executor::plugin_trait::RouterPluginBoxed;
use http::StatusCode;
use moka::future::Cache;
use moka::Expiry;
use ntex::web;
use ntex::{http::HeaderMap, util::Bytes};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{collections::HashSet, sync::Arc};

use crate::cache_state::CacheState;
use crate::jwt::context::JwtTokenPayload;
use crate::jwt::JwtAuthRuntime;
use crate::pipeline::cors::{CORSConfigError, Cors};
use crate::pipeline::introspection_policy::compile_introspection_policy;
use crate::pipeline::parser::ParseCacheEntry;
use crate::pipeline::progressive_override::{OverrideLabelsCompileError, OverrideLabelsEvaluator};

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

#[derive(Clone)]
pub struct SharedRouterResponse {
    pub body: Bytes,
    pub headers: Arc<HeaderMap>,
    pub status: StatusCode,
    pub error_count: usize,
}

impl From<SharedRouterResponse> for web::HttpResponse {
    fn from(shared_response: SharedRouterResponse) -> Self {
        let mut response = web::HttpResponse::Ok();
        response.status(shared_response.status);

        for (header_name, header_value) in shared_response.headers.iter() {
            response.set_header(header_name, header_value);
        }

        response.body(shared_response.body)
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
    pub plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
    pub in_flight_requests: RouterInflightRequestsMap,
    pub in_flight_requests_header_policy: RouterRequestDedupeHeaderPolicy,
}

impl RouterSharedState {
    pub fn new(
        router_config: Arc<HiveRouterConfig>,
        jwt_auth_runtime: Option<JwtAuthRuntime>,
        hive_usage_agent: Option<UsageAgent>,
        validation_plan: ValidationPlan,
        telemetry_context: Arc<TelemetryContext>,
        plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
        cache_state: Arc<CacheState>,
    ) -> Result<Self, SharedStateError> {
        let parse_cache = cache_state.parse_cache.clone();
        Ok(Self {
            validation_plan: Arc::new(validation_plan),
            headers_plan: Arc::new(compile_headers_plan(&router_config.headers).map_err(Box::new)?),
            parse_cache,
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
            plugins,
            in_flight_requests: InFlightMap::default(),
            in_flight_requests_header_policy: (&router_config
                .traffic_shaping
                .router
                .dedupe
                .headers)
                .into(),
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
    #[error("invalid introspection config: {0}")]
    IntrospectionPolicyCompile(#[from] Box<ExpressionCompileError>),
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
