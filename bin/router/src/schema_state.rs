use arc_swap::{ArcSwap, Guard};
use async_trait::async_trait;
use graphql_tools::parser::schema::Document;
use graphql_tools::validation::utils::ValidationError;
use hive_router_config::{supergraph::SupergraphSource, HiveRouterConfig};
use hive_router_internal::telemetry::TelemetryContext;
use hive_router_plan_executor::{
    executors::error::SubgraphExecutorError,
    introspection::schema::{SchemaMetadata, SchemaWithMetadata},
    SubgraphExecutorMap,
};
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use hive_router_query_planner::{
    planner::{Planner, PlannerError},
    state::supergraph_state::SupergraphState,
    utils::parsing::parse_schema,
};
use moka::future::Cache;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace};

use crate::{
    background_tasks::{BackgroundTask, BackgroundTasksManager},
    pipeline::{
        authorization::{AuthorizationMetadata, AuthorizationMetadataError},
        normalize::GraphQLNormalizationPayload,
    },
    supergraph::{
        base::{LoadSupergraphError, ReloadSupergraphResult, SupergraphLoader},
        resolve_from_config,
    },
};

pub struct SchemaState {
    current_swapable: Arc<ArcSwap<Option<SupergraphData>>>,
    pub plan_cache: Cache<u64, Arc<QueryPlan>>,
    pub validate_cache: Cache<u64, Arc<Vec<ValidationError>>>,
    pub normalize_cache: Cache<u64, Arc<GraphQLNormalizationPayload>>,
}

pub struct SupergraphData {
    pub metadata: SchemaMetadata,
    pub planner: Planner,
    pub authorization: AuthorizationMetadata,
    pub subgraph_executor_map: SubgraphExecutorMap,
    pub supergraph_schema: Arc<Document<'static, String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum SupergraphManagerError {
    #[error("Failed to load supergraph: {0}")]
    LoadSupergraphError(#[from] LoadSupergraphError),

    #[error("Failed to build planner: {0}")]
    PlannerBuilderError(#[from] PlannerError),
    #[error("Failed to build authorization: {0}")]
    AuthorizationMetadataError(#[from] AuthorizationMetadataError),
    #[error(transparent)]
    ExecutorInitError(#[from] SubgraphExecutorError),
    #[error("Unexpected: failed to load initial supergraph")]
    FailedToLoadInitialSupergraph,
}

impl SchemaState {
    pub fn current_supergraph(&self) -> Guard<Arc<Option<SupergraphData>>> {
        self.current_swapable.load()
    }

    pub fn is_ready(&self) -> bool {
        self.current_supergraph().is_some()
    }

    pub async fn new_from_config(
        bg_tasks_manager: &mut BackgroundTasksManager,
        telemetry_context: Arc<TelemetryContext>,
        router_config: Arc<HiveRouterConfig>,
    ) -> Result<Self, SupergraphManagerError> {
        let (tx, mut rx) = mpsc::channel::<String>(1);
        let background_loader = SupergraphBackgroundLoader::new(&router_config.supergraph, tx)?;
        bg_tasks_manager.register_task(Arc::new(background_loader));

        let swappable_data = Arc::new(ArcSwap::from(Arc::new(None)));
        let swappable_data_spawn_clone = swappable_data.clone();
        let plan_cache = Cache::new(1000);
        let validate_cache = Cache::new(1000);
        let normalize_cache = Cache::new(1000);

        // This is cheap clone, as Cache is thread-safe and can be cloned without any performance penalty.
        let task_plan_cache = plan_cache.clone();
        let validate_cache_cache = validate_cache.clone();
        let normalize_cache_cache = normalize_cache.clone();

        bg_tasks_manager.register_handle(async move {
            while let Some(new_sdl) = rx.recv().await {
                debug!("Received new supergraph SDL, building new supergraph state...");

                match Self::build_data(router_config.clone(), telemetry_context.clone(), &new_sdl) {
                    Ok(new_data) => {
                        swappable_data_spawn_clone.store(Arc::new(Some(new_data)));
                        info!("Supergraph updated successfully, will be used for next request, clearing caches...");
                        task_plan_cache.invalidate_all();
                        validate_cache_cache.invalidate_all();
                        normalize_cache_cache.invalidate_all();
                        debug!("Schema-associated caches cleared successfully");
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to build new supergraph data");
                    }
                }
            }
        });

        Ok(Self {
            current_swapable: swappable_data,
            plan_cache,
            validate_cache,
            normalize_cache,
        })
    }

    fn build_data(
        router_config: Arc<HiveRouterConfig>,
        telemetry_context: Arc<TelemetryContext>,
        supergraph_sdl: &str,
    ) -> Result<SupergraphData, SupergraphManagerError> {
        let parsed_supergraph_sdl = parse_schema(supergraph_sdl);
        let supergraph_state = SupergraphState::new(&parsed_supergraph_sdl);
        let planner = Planner::new_from_supergraph(&parsed_supergraph_sdl)?;
        let metadata = planner.consumer_schema.schema_metadata();
        let authorization = AuthorizationMetadata::build(&planner.supergraph, &metadata)?;
        let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(
            &supergraph_state.subgraph_endpoint_map,
            router_config,
            telemetry_context,
        )?;

        Ok(SupergraphData {
            supergraph_schema: Arc::new(parsed_supergraph_sdl),
            metadata,
            planner,
            authorization,
            subgraph_executor_map,
        })
    }
}

pub struct SupergraphBackgroundLoader {
    loader: Box<dyn SupergraphLoader + Send + Sync>,
    sender: Arc<mpsc::Sender<String>>,
}

impl SupergraphBackgroundLoader {
    pub fn new(
        config: &SupergraphSource,
        sender: mpsc::Sender<String>,
    ) -> Result<Self, LoadSupergraphError> {
        let loader = resolve_from_config(config)?;

        Ok(Self {
            loader,
            sender: Arc::new(sender),
        })
    }
}

#[async_trait]
impl BackgroundTask for Arc<SupergraphBackgroundLoader> {
    fn id(&self) -> &str {
        "supergraph-background-loader"
    }

    async fn run(&self, token: CancellationToken) {
        loop {
            if token.is_cancelled() {
                trace!("Background task cancelled");

                break;
            }

            match self.loader.load().await {
                Ok(ReloadSupergraphResult::Unchanged) => {
                    debug!("Supergraph fetched successfully with no changes");
                }
                Ok(ReloadSupergraphResult::Changed { new_sdl }) => {
                    debug!("Supergraph loaded successfully with changes, updating...");

                    if self.sender.clone().send(new_sdl).await.is_err() {
                        error!("Failed to send new supergraph SDL: receiver dropped.");
                        break;
                    }
                }
                Err(err) => {
                    error!("Failed to load supergraph: {}", err);
                }
            }

            if let Some(interval) = self.loader.reload_interval() {
                debug!(
                    "waiting for {:?}ms before checking again for supergraph changes",
                    interval.as_millis()
                );

                ntex::time::sleep(*interval).await;
            } else {
                debug!("poll interval not configured for supergraph changes, breaking");

                break;
            }
        }
    }
}
