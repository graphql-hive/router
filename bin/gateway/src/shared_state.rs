use hashbrown::HashMap;
use std::sync::Arc;

use graphql_parser::schema::Document;
use graphql_tools::validation::{utils::ValidationError, validate::ValidationPlan};
use moka::future::Cache;
use query_plan_executor::executors::map::SubgraphExecutorMap;
use query_plan_executor::schema_metadata::{SchemaMetadata, SchemaWithMetadata};
use query_planner::{
    planner::{plan_nodes::QueryPlan, Planner},
    state::supergraph_state::SupergraphState,
};

pub struct GatewaySharedState {
    pub schema_metadata: SchemaMetadata,
    pub planner: Planner,
    pub validation_plan: ValidationPlan,
    pub subgraph_executor_map: SubgraphExecutorMap,
    pub subgraph_endpoint_map: HashMap<String, String>,
    pub plan_cache: Cache<u64, Arc<QueryPlan>>,
    pub validate_cache: Cache<u64, Arc<Vec<ValidationError>>>,
}

impl GatewaySharedState {
    pub fn new(parsed_supergraph_sdl: Document<'static, String>) -> Arc<Self> {
        let supergraph_state = SupergraphState::new(&parsed_supergraph_sdl);
        let planner =
            Planner::new_from_supergraph(&parsed_supergraph_sdl).expect("failed to create planner");
        let schema_metadata = planner.consumer_schema.schema_metadata();

        let subgraph_endpoint_map = supergraph_state.subgraph_endpoint_map.clone();
        let subgraph_executor_map =
            SubgraphExecutorMap::from_http_endpoint_map(supergraph_state.subgraph_endpoint_map);

        Arc::new(Self {
            schema_metadata,
            planner,
            validation_plan: graphql_tools::validation::rules::default_rules_validation_plan(),
            subgraph_executor_map,
            subgraph_endpoint_map,
            plan_cache: moka::future::Cache::new(1000),
            validate_cache: moka::future::Cache::new(1000),
        })
    }
}
