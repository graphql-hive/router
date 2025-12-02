use arc_swap::{ArcSwap, Guard};
use async_trait::async_trait;
use graphql_tools::{static_graphql::schema::Document, validation::utils::ValidationError};
use hive_router_config::{supergraph::SupergraphSource, HiveRouterConfig};
use hive_router_plan_executor::{
    executors::error::SubgraphExecutorError,
    hooks::on_supergraph_load::{
        OnSupergraphLoadEndHookPayload, OnSupergraphLoadStartHookPayload, SupergraphData,
    },
    introspection::schema::SchemaWithMetadata,
    plugin_trait::{EndControlFlow, StartControlFlow},
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
use tracing::{debug, error, trace};

use crate::{
    background_tasks::{BackgroundTask, BackgroundTasksManager},
    pipeline::normalize::GraphQLNormalizationPayload,
    supergraph::{
        base::{LoadSupergraphError, ReloadSupergraphResult, SupergraphLoader},
        resolve_from_config,
    },
    RouterSharedState,
};

pub struct SchemaState {
    current_swapable: Arc<ArcSwap<Option<SupergraphData>>>,
    pub plan_cache: Cache<u64, Arc<QueryPlan>>,
    pub validate_cache: Cache<u64, Arc<Vec<ValidationError>>>,
    pub normalize_cache: Cache<u64, Arc<GraphQLNormalizationPayload>>,
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

impl SchemaState {
    pub fn current_supergraph(&self) -> Guard<Arc<Option<SupergraphData>>> {
        self.current_swapable.load()
    }

    pub fn is_ready(&self) -> bool {
        self.current_supergraph().is_some()
    }

    pub async fn new_from_config(
        bg_tasks_manager: &mut BackgroundTasksManager,
        router_config: Arc<HiveRouterConfig>,
        app_state: Arc<RouterSharedState>,
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

                let mut new_ast = parse_schema(&new_sdl);

                let mut on_end_callbacks = vec![];

                if let Some(plugins) = app_state.plugins.as_ref() {
                    let mut start_payload = OnSupergraphLoadStartHookPayload {
                        current_supergraph_data: swappable_data_spawn_clone.clone(),
                        new_ast,
                    };
                    for plugin in plugins.as_ref() {
                        let result = plugin.on_supergraph_reload(start_payload);
                        start_payload = result.payload;
                        match result.control_flow {
                            StartControlFlow::Continue => {
                                // continue to next plugin
                            }
                            StartControlFlow::EndResponse(_) => {
                                unreachable!("Plugins should not end supergraph reload processing");
                            }
                            StartControlFlow::OnEnd(callback) => {
                                on_end_callbacks.push(callback);
                            }
                        }
                    }
                    new_ast = start_payload.new_ast;
                }

                match Self::build_data(router_config.clone(), &new_ast) {
                    Ok(new_supergraph_data) => {
                        let mut end_payload = OnSupergraphLoadEndHookPayload {
                            new_supergraph_data,
                        };

                        for callback in on_end_callbacks {
                            let result = callback(end_payload);
                            end_payload = result.payload;
                            match result.control_flow {
                                EndControlFlow::Continue => {
                                    // continue to next callback
                                }
                                EndControlFlow::EndResponse(_) => {
                                    unreachable!(
                                        "Plugins should not end supergraph reload processing"
                                    );
                                }
                            }
                        }

                        let new_supergraph_data = end_payload.new_supergraph_data;

                        swappable_data_spawn_clone.store(Arc::new(Some(new_supergraph_data)));
                        debug!("Supergraph updated successfully");

                        task_plan_cache.invalidate_all();
                        validate_cache_cache.invalidate_all();
                        normalize_cache_cache.invalidate_all();
                        debug!("Schema-associated caches cleared successfully");
                    }
                    Err(e) => {
                        error!("Failed to build new supergraph data: {}", e);
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
        parsed_supergraph_sdl: &Document,
    ) -> Result<SupergraphData, SupergraphManagerError> {
        let supergraph_state = SupergraphState::new(parsed_supergraph_sdl);
        let planner = Planner::new_from_supergraph(parsed_supergraph_sdl)?;
        let metadata = planner.consumer_schema.schema_metadata();
        let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(
            supergraph_state.subgraph_endpoint_map,
            router_config,
        )?;

        Ok(SupergraphData {
            metadata,
            planner,
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
impl BackgroundTask for SupergraphBackgroundLoader {
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
