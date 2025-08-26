use std::sync::Arc;

use executor::{
    introspection::schema::{SchemaMetadata, SchemaWithMetadata},
    SubgraphExecutorMap,
};
use gateway_config::HiveRouterConfig;
use graphql_parser::schema::Document;
use graphql_tools::validation::{utils::ValidationError, validate::ValidationPlan};
use moka::future::Cache;
use query_planner::{
    planner::{plan_nodes::QueryPlan, Planner},
    state::supergraph_state::SupergraphState,
};

use crate::pipeline::normalize::GraphQLNormalizationPayload;

pub struct GatewaySharedState {
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

impl GatewaySharedState {
    pub fn new(
        parsed_supergraph_sdl: Document<'static, String>,
        router_config: HiveRouterConfig,
    ) -> Arc<Self> {
        let supergraph_state = SupergraphState::new(&parsed_supergraph_sdl);
        let planner =
            Planner::new_from_supergraph(&parsed_supergraph_sdl).expect("failed to create planner");
        let schema_metadata = planner.consumer_schema.schema_metadata();

        let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(
            supergraph_state.subgraph_endpoint_map,
            router_config.traffic_shaping.clone(),
        )
        .expect("Failed to create subgraph executor map");

        Arc::new(Self {
            schema_metadata,
            planner,
            validation_plan: graphql_tools::validation::rules::default_rules_validation_plan(),
            subgraph_executor_map,
            plan_cache: moka::future::Cache::new(1000),
            validate_cache: moka::future::Cache::new(1000),
            parse_cache: moka::future::Cache::new(1000),
            normalize_cache: moka::future::Cache::new(1000),
            router_config,
        })
    }
}
