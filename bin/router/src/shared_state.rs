use graphql_tools::validation::validate::ValidationPlan;
use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::headers::{
    compile::compile_headers_plan, errors::HeaderRuleCompileError, plan::HeaderRulesPlan,
};
use moka::future::Cache;
use moka::Expiry;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::jwt::context::JwtTokenPayload;
use crate::jwt::JwtAuthRuntime;
use crate::pipeline::cors::{CORSConfigError, Cors};
use crate::pipeline::progressive_override::{OverrideLabelsCompileError, OverrideLabelsEvaluator};

pub type JwtClaimsCache = Cache<String, Arc<JwtTokenPayload>>;

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
    pub validation_plan: ValidationPlan,
    pub parse_cache: Cache<u64, Arc<graphql_parser::query::Document<'static, String>>>,
    pub router_config: Arc<HiveRouterConfig>,
    pub headers_plan: HeaderRulesPlan,
    pub override_labels_evaluator: OverrideLabelsEvaluator,
    pub cors_runtime: Option<Cors>,
    /// Cache for validated JWT claims to avoid re-parsing on every request.
    /// The cache key is the raw JWT token string.
    /// Stores the parsed claims payload for 5s,
    /// but no longer than `exp` date.
    pub jwt_claims_cache: JwtClaimsCache,
    pub jwt_auth_runtime: Option<JwtAuthRuntime>,
}

impl RouterSharedState {
    pub fn new(
        router_config: Arc<HiveRouterConfig>,
        jwt_auth_runtime: Option<JwtAuthRuntime>,
    ) -> Result<Self, SharedStateError> {
        Ok(Self {
            validation_plan: graphql_tools::validation::rules::default_rules_validation_plan(),
            headers_plan: compile_headers_plan(&router_config.headers).map_err(Box::new)?,
            parse_cache: moka::future::Cache::new(1000),
            cors_runtime: Cors::from_config(&router_config.cors).map_err(Box::new)?,
            jwt_claims_cache: Cache::builder()
                // Consistent with parse_cache and prevents unbounded memory usage.
                .max_capacity(1000)
                .expire_after(JwtClaimsExpiry)
                .build(),
            router_config: router_config.clone(),
            override_labels_evaluator: OverrideLabelsEvaluator::from_config(
                &router_config.override_labels,
            )
            .map_err(Box::new)?,
            jwt_auth_runtime,
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
}
