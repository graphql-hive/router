use std::{collections::HashMap, sync::Arc};

use executor::{
    executors::config::HttpExecutorConfig,
    introspection::schema::{SchemaMetadata, SchemaWithMetadata},
    SubgraphExecutorMap,
};
use graphql_parser::schema::Document;
use graphql_tools::validation::{utils::ValidationError, validate::ValidationPlan};
use moka::future::Cache;
use query_planner::{
    planner::{plan_nodes::QueryPlan, Planner},
    state::supergraph_state::SupergraphState,
};

use crate::pipeline::normalize_service::GraphQLNormalizationPayload;

pub struct GatewaySharedState {
    pub schema_metadata: SchemaMetadata,
    pub planner: Planner,
    pub validation_plan: ValidationPlan,
    pub subgraph_executor_map: SubgraphExecutorMap,
    pub subgraph_endpoint_map: HashMap<String, String>,
    pub plan_cache: Cache<u64, Arc<QueryPlan>>,
    pub validate_cache: Cache<u64, Arc<Vec<ValidationError>>>,
    pub parse_cache: Cache<u64, Arc<graphql_parser::query::Document<'static, String>>>,
    pub normalize_cache: Cache<u64, Arc<GraphQLNormalizationPayload>>,
    pub supergraph_version: String,
    pub sdl: String,
}

impl GatewaySharedState {
    pub fn new(
        parsed_supergraph_sdl: Document<'static, String>,
        supergraph_version: String,
    ) -> Arc<Self> {
        let supergraph_state = SupergraphState::new(&parsed_supergraph_sdl);
        let planner =
            Planner::new_from_supergraph(&parsed_supergraph_sdl).expect("failed to create planner");
        let schema_metadata = planner.consumer_schema.schema_metadata();

        let subgraph_endpoint_map = supergraph_state.subgraph_endpoint_map.clone();
        let http_executor_config = HttpExecutorConfig::default();
        let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(
            supergraph_state.subgraph_endpoint_map,
            http_executor_config,
        )
        .expect("Failed to create subgraph executor map");

        Arc::new(Self {
            schema_metadata,
            planner,
            validation_plan: graphql_tools::validation::rules::default_rules_validation_plan(),
            subgraph_executor_map,
            subgraph_endpoint_map,
            plan_cache: moka::future::Cache::new(1000),
            validate_cache: moka::future::Cache::new(1000),
            parse_cache: moka::future::Cache::new(1000),
            normalize_cache: moka::future::Cache::new(1000),
            supergraph_version,
            sdl: parsed_supergraph_sdl.to_string(),
        })
    }
}
