use std::sync::Arc;

use graphql_parser::schema::Document;
use graphql_tools::validation::{utils::ValidationError, validate::ValidationPlan};
use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::{
    headers::{
        compile::compile_headers_plan, errors::HeaderRuleCompileError, plan::HeaderRulesPlan,
    },
    introspection::schema::{SchemaMetadata, SchemaWithMetadata},
    SubgraphExecutorMap,
};
use hive_router_query_planner::{
    planner::{plan_nodes::QueryPlan, Planner},
    state::supergraph_state::SupergraphState,
};
use moka::future::Cache;

use crate::pipeline::{
    cors::{CORSConfigError, Cors},
    normalize::GraphQLNormalizationPayload,
};

pub struct RouterSharedState {
    pub schema_metadata: SchemaMetadata,
    pub planner: Planner,
    pub validation_plan: ValidationPlan,
    pub subgraph_executor_map: SubgraphExecutorMap,
    pub plan_cache: Cache<u64, Arc<QueryPlan>>,
    pub validate_cache: Cache<u64, Arc<Vec<ValidationError>>>,
    pub parse_cache: Cache<u64, Arc<graphql_parser::query::Document<'static, String>>>,
    pub normalize_cache: Cache<u64, Arc<GraphQLNormalizationPayload>>,
    pub router_config: HiveRouterConfig,
    pub headers_plan: HeaderRulesPlan,
    pub cors: Option<Cors>,
}

impl RouterSharedState {
    pub fn new(
        parsed_supergraph_sdl: Document<'static, String>,
        router_config: HiveRouterConfig,
    ) -> Result<Arc<Self>, SharedStateError> {
        let supergraph_state = SupergraphState::new(&parsed_supergraph_sdl);
        let planner =
            Planner::new_from_supergraph(&parsed_supergraph_sdl).expect("failed to create planner");
        let schema_metadata = planner.consumer_schema.schema_metadata();

        let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(
            supergraph_state.subgraph_endpoint_map,
            router_config.override_subgraph_urls.clone(),
            router_config.traffic_shaping.clone(),
        )
        .expect("Failed to create subgraph executor map");

        Ok(Arc::new(Self {
            schema_metadata,
            planner,
            validation_plan: graphql_tools::validation::rules::default_rules_validation_plan(),
            headers_plan: compile_headers_plan(&router_config.headers).map_err(Box::new)?,
            subgraph_executor_map,
            plan_cache: moka::future::Cache::new(1000),
            validate_cache: moka::future::Cache::new(1000),
            parse_cache: moka::future::Cache::new(1000),
            normalize_cache: moka::future::Cache::new(1000),
            cors: Cors::from_config(&router_config.cors).map_err(Box::new)?,
            router_config,
        }))
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SharedStateError {
    #[error("invalid headers config: {0}")]
    HeaderRuleCompileError(#[from] Box<HeaderRuleCompileError>),
    #[error("invalid regex in CORS config: {0}")]
    CORSConfigError(#[from] Box<CORSConfigError>),
}
