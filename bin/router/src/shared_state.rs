use graphql_tools::validation::{utils::ValidationError, validate::ValidationPlan};
use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::headers::{
    compile::compile_headers_plan, errors::HeaderRuleCompileError, plan::HeaderRulesPlan,
};
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use moka::future::Cache;
use std::sync::Arc;

use crate::jwt::JwtAuthRuntime;
use crate::pipeline::{
    cors::{CORSConfigError, Cors},
    normalize::GraphQLNormalizationPayload,
};

pub struct RouterSharedState {
    pub validation_plan: ValidationPlan,
    pub plan_cache: Cache<u64, Arc<QueryPlan>>,
    pub validate_cache: Cache<u64, Arc<Vec<ValidationError>>>,
    pub parse_cache: Cache<u64, Arc<graphql_parser::query::Document<'static, String>>>,
    pub normalize_cache: Cache<u64, Arc<GraphQLNormalizationPayload>>,
    pub router_config: HiveRouterConfig,
    pub headers_plan: HeaderRulesPlan,
    pub cors: Option<Cors>,
    pub jwt_auth_runtime: Option<JwtAuthRuntime>,
}

impl RouterSharedState {
    pub fn new(
        router_config: HiveRouterConfig,
        jwt_auth_runtime: Option<JwtAuthRuntime>,
    ) -> Result<Self, SharedStateError> {
        Ok(Self {
            validation_plan: graphql_tools::validation::rules::default_rules_validation_plan(),
            headers_plan: compile_headers_plan(&router_config.headers).map_err(Box::new)?,
            plan_cache: moka::future::Cache::new(1000),
            validate_cache: moka::future::Cache::new(1000),
            parse_cache: moka::future::Cache::new(1000),
            normalize_cache: moka::future::Cache::new(1000),
            cors: Cors::from_config(&router_config.cors).map_err(Box::new)?,
            router_config,
            jwt_auth_runtime,
        })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SharedStateError {
    #[error("invalid headers config: {0}")]
    HeaderRuleCompileError(#[from] Box<HeaderRuleCompileError>),
    #[error("invalid regex in CORS config: {0}")]
    CORSConfigError(#[from] Box<CORSConfigError>),
}
