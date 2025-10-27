use graphql_tools::validation::validate::ValidationPlan;
use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::headers::{
    compile::compile_headers_plan, errors::HeaderRuleCompileError, plan::HeaderRulesPlan,
};
use moka::future::Cache;
use std::sync::Arc;

use crate::jwt::JwtAuthRuntime;
use crate::pipeline::cors::{CORSConfigError, Cors};
use crate::pipeline::progressive_override::{OverrideLabelsCompileError, OverrideLabelsEvaluator};

pub struct RouterSharedState {
    pub validation_plan: ValidationPlan,
    pub parse_cache: Cache<u64, Arc<graphql_parser::query::Document<'static, String>>>,
    pub router_config: Arc<HiveRouterConfig>,
    pub headers_plan: HeaderRulesPlan,
    pub override_labels_evaluator: OverrideLabelsEvaluator,
    pub cors_runtime: Option<Cors>,
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
