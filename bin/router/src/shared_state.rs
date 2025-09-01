use std::sync::Arc;

use graphql_tools::validation::{utils::ValidationError, validate::ValidationPlan};
use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::{
    executors::error::SubgraphExecutorError,
    introspection::schema::{SchemaMetadata, SchemaWithMetadata},
    SubgraphExecutorMap,
};
use hive_router_query_planner::{
    planner::{plan_nodes::QueryPlan, Planner, PlannerError},
    state::supergraph_state::SupergraphState,
    utils::parsing::parse_schema,
};
use moka::future::Cache;

use crate::{
    pipeline::normalize::GraphQLNormalizationPayload,
    supergraph::{base::LoadSupergraphError, resolve_from_config},
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
}

#[derive(Debug, thiserror::Error)]
pub enum RouterSharedStateError {
    #[error("Failed to load supergraph: {0}")]
    SupergraphInitFailure(#[from] LoadSupergraphError),
    #[error("Failed to init planner: {0}")]
    PlannerInitError(#[from] PlannerError),
    #[error("Failed to init executor: {0}")]
    ExecutorInitError(#[from] SubgraphExecutorError),
}

impl RouterSharedState {
    pub async fn new(router_config: HiveRouterConfig) -> Result<Arc<Self>, RouterSharedStateError> {
        let mut supergraph_source_loader = resolve_from_config(&router_config.supergraph).await?;
        supergraph_source_loader.reload().await?;
        let supergraph_sdl = supergraph_source_loader
            .current()
            .expect("supergraph should be available after a successful reload");
        let parsed_supergraph_sdl = parse_schema(supergraph_sdl);
        let supergraph_state = SupergraphState::new(&parsed_supergraph_sdl);
        let planner = Planner::new_from_supergraph(&parsed_supergraph_sdl)?;
        let schema_metadata = planner.consumer_schema.schema_metadata();

        let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(
            supergraph_state.subgraph_endpoint_map,
            router_config.traffic_shaping.clone(),
        )?;

        Ok(Arc::new(Self {
            schema_metadata,
            planner,
            validation_plan: graphql_tools::validation::rules::default_rules_validation_plan(),
            subgraph_executor_map,
            plan_cache: moka::future::Cache::new(1000),
            validate_cache: moka::future::Cache::new(1000),
            parse_cache: moka::future::Cache::new(1000),
            normalize_cache: moka::future::Cache::new(1000),
            router_config,
        }))
    }
}
