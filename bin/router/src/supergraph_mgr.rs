use std::sync::Arc;

use arc_swap::{ArcSwap, Guard};
use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::{
    executors::error::SubgraphExecutorError,
    introspection::schema::{SchemaMetadata, SchemaWithMetadata},
    SubgraphExecutorMap,
};
use hive_router_query_planner::{
    planner::{Planner, PlannerError},
    state::supergraph_state::SupergraphState,
    utils::parsing::parse_schema,
};

use crate::supergraph::{
    base::{LoadSupergraphError, SupergraphLoader},
    resolve_from_config,
};

pub struct SupergraphManager {
    current: ArcSwap<SupergraphData>,
    #[allow(dead_code)]
    loader: Box<dyn SupergraphLoader + Send + Sync>,
}

pub struct SupergraphData {
    pub metadata: SchemaMetadata,
    pub planner: Planner,
    pub subgraph_executor_map: SubgraphExecutorMap,
}

#[derive(Debug, thiserror::Error)]
pub enum SupergraphManagerError {
    #[error("Failed to load supergraph: {0}")]
    LoadSupergraphError(#[from] LoadSupergraphError),
    #[error("Failed to build planner: {0}")]
    PlannerBuilderError(#[from] PlannerError),
    #[error("Failed to init executor: {0}")]
    ExecutorInitError(#[from] SubgraphExecutorError),
    #[error("Unexpected: failed to load initial supergraph")]
    FailedToLoadInitialSupergraph,
}

impl SupergraphManager {
    pub async fn new_from_config(
        router_config: &HiveRouterConfig,
    ) -> Result<Self, SupergraphManagerError> {
        let mut loader = resolve_from_config(&router_config.supergraph).await?;
        loader.reload().await?;
        let supergraph_sdl = loader
            .current()
            .ok_or_else(|| SupergraphManagerError::FailedToLoadInitialSupergraph)?;
        let current_data = Self::build_data(router_config, supergraph_sdl)?;
        let swappable_data = ArcSwap::from(Arc::new(current_data));

        Ok(Self {
            current: swappable_data,
            loader,
        })
    }

    fn build_data(
        router_config: &HiveRouterConfig,
        supergraph_sdl: &str,
    ) -> Result<SupergraphData, SupergraphManagerError> {
        let parsed_supergraph_sdl = parse_schema(supergraph_sdl);
        let supergraph_state = SupergraphState::new(&parsed_supergraph_sdl);
        let planner = Planner::new_from_supergraph(&parsed_supergraph_sdl)?;
        let metadata = planner.consumer_schema.schema_metadata();
        let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(
            supergraph_state.subgraph_endpoint_map,
            router_config.traffic_shaping.clone(),
        )?;

        Ok(SupergraphData {
            metadata,
            planner,
            subgraph_executor_map,
        })
    }

    pub fn current(&self) -> Guard<Arc<SupergraphData>> {
        self.current.load()
    }
}
