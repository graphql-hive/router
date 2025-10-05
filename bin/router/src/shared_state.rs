use std::sync::Arc;

use graphql_tools::validation::{utils::ValidationError, validate::ValidationPlan};
use hive_router_config::HiveRouterConfig;
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use moka::future::Cache;

use crate::pipeline::normalize::GraphQLNormalizationPayload;

pub struct RouterSharedState {
    pub validation_plan: ValidationPlan,
    pub plan_cache: Cache<u64, Arc<QueryPlan>>,
    pub validate_cache: Cache<u64, Arc<Vec<ValidationError>>>,
    pub parse_cache: Cache<u64, Arc<graphql_parser::query::Document<'static, String>>>,
    pub normalize_cache: Cache<u64, Arc<GraphQLNormalizationPayload>>,
    pub router_config: HiveRouterConfig,
}

impl RouterSharedState {
    pub fn new(router_config: HiveRouterConfig) -> Self {
        Self {
            validation_plan: graphql_tools::validation::rules::default_rules_validation_plan(),
            plan_cache: moka::future::Cache::new(1000),
            validate_cache: moka::future::Cache::new(1000),
            parse_cache: moka::future::Cache::new(1000),
            normalize_cache: moka::future::Cache::new(1000),
            router_config,
        }
    }
}
