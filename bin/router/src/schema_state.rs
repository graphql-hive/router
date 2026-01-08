use crate::pipeline::authorization::metadata::AuthorizationMetadataExt;
use arc_swap::{ArcSwap, Guard};
use async_trait::async_trait;
use graphql_tools::static_graphql::schema::Document;
use graphql_tools::validation::utils::ValidationError;
use hive_router_config::{supergraph::SupergraphSource, HiveRouterConfig};
use hive_router_internal::telemetry::{metrics::Metrics, TelemetryContext};
use hive_router_internal::{
    authorization::metadata::AuthorizationMetadata,
    background_tasks::{BackgroundTask, BackgroundTasksManager},
};
use hive_router_plan_executor::{
    executors::error::SubgraphExecutorError,
    hooks::on_supergraph_load::{
        OnSupergraphLoadEndHookPayload, OnSupergraphLoadStartHookPayload, SupergraphData,
    },
    introspection::schema::SchemaWithMetadata,
    plugin_trait::{EndControlFlow, RouterPluginBoxed, StartControlFlow},
    SubgraphExecutorMap,
};
use hive_router_query_planner::planner::plan_nodes::QueryPlan;
use hive_router_query_planner::{
    planner::{Planner, PlannerError},
    utils::parsing::parse_schema,
};
use moka::future::Cache;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, trace};

use crate::{
    cache_state::CacheState,
    pipeline::{authorization::AuthorizationMetadataError, normalize::GraphQLNormalizationPayload},
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
    pub telemetry_context: Arc<TelemetryContext>,
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

    #[error("Error from plugin: {0}")]
    PluginError(String),
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
        plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
        cache_state: Arc<CacheState>,
    ) -> Result<Self, SupergraphManagerError> {
        let (tx, mut rx) = mpsc::channel::<String>(1);
        let background_loader = SupergraphBackgroundLoader::new(
            &router_config.supergraph,
            tx,
            telemetry_context.metrics.clone(),
        )?;
        bg_tasks_manager.register_task(SupergraphBackgroundLoaderTask(Arc::new(background_loader)));

        let swappable_data = Arc::new(ArcSwap::from(Arc::new(None)));
        let swappable_data_spawn_clone = swappable_data.clone();
        let plan_cache = cache_state.plan_cache.clone();
        let validate_cache = cache_state.validate_cache.clone();
        let normalize_cache = cache_state.normalize_cache.clone();

        // This is cheap clone, as Cache is thread-safe and can be cloned without any performance penalty.
        let cache_state_for_invalidation = cache_state.clone();

        let metrics = telemetry_context.metrics.clone();
        let task_telemetry = telemetry_context.clone();
        bg_tasks_manager.register_handle(async move {
            let supergraph_metrics = &metrics.supergraph;
            while let Some(new_sdl) = rx.recv().await {
                let process_capture = supergraph_metrics.capture_process();
                debug!("Received new supergraph SDL, building new supergraph state...");

                let mut new_ast = parse_schema(&new_sdl);

                let mut on_end_callbacks = vec![];

                let mut new_supergraph_data = None;
                if let Some(plugins) = plugins.as_ref() {
                    let current_supergraph_data = swappable_data_spawn_clone.load().clone();
                    let mut start_payload = OnSupergraphLoadStartHookPayload {
                        current_supergraph_data,
                        new_ast,
                    };
                    for plugin in plugins.as_ref() {
                        let result = plugin.on_supergraph_reload(start_payload);
                        start_payload = result.payload;
                        match result.control_flow {
                            StartControlFlow::Proceed => {
                                // continue to next plugin
                            }
                            // There is no way to end with response here, so we treat it as error
                            // or the way to override the supergraph data
                            StartControlFlow::EndWithResponse(plugin_res) => {
                                new_supergraph_data = Some(plugin_res.map_err(|err| {
                                    SupergraphManagerError::PluginError(err.message)
                                }));
                                break;
                            }
                            StartControlFlow::OnEnd(callback) => {
                                on_end_callbacks.push(callback);
                            }
                        }
                    }
                    // Give the ownership back to variables
                    new_ast = start_payload.new_ast;
                }

                match new_supergraph_data.unwrap_or_else(|| {
                    Self::build_data(router_config.clone(), task_telemetry.clone(), new_ast)
                }) {
                    Ok(mut new_supergraph_data) => {
                        if !on_end_callbacks.is_empty() {
                            let mut end_payload = OnSupergraphLoadEndHookPayload {
                                new_supergraph_data,
                            };

                            for callback in on_end_callbacks {
                                let result = callback(end_payload);
                                end_payload = result.payload;
                                match result.control_flow {
                                    EndControlFlow::Proceed => {
                                        // continue to next callback
                                    }
                                    // Similar to StartControlFlow,
                                    // There is no way to end with response here, so we treat it as error
                                    // or the way to override the supergraph data
                                    EndControlFlow::EndWithResponse(plugin_res) => match plugin_res
                                    {
                                        Ok(data) => {
                                            end_payload.new_supergraph_data = data;
                                        }
                                        Err(err) => {
                                            process_capture.finish_error();
                                            error!(
                                                "Plugin ended supergraph load with error: {}",
                                                err.message
                                            );
                                            return;
                                        }
                                    },
                                }
                            }

                            // Give the ownership back to new_supergraph_data
                            new_supergraph_data = end_payload.new_supergraph_data;
                        }

                        swappable_data_spawn_clone.store(Arc::new(Some(new_supergraph_data)));
                        debug!("Supergraph updated successfully");

                        cache_state_for_invalidation.on_schema_change();
                        debug!("Schema-associated caches cleared successfully");
                        process_capture.finish_ok();
                    }
                    Err(e) => {
                        process_capture.finish_error();
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
            telemetry_context: telemetry_context.clone(),
        })
    }

    fn build_data(
        router_config: Arc<HiveRouterConfig>,
        telemetry_context: Arc<TelemetryContext>,
        parsed_supergraph_sdl: Document,
    ) -> Result<SupergraphData, SupergraphManagerError> {
        let planner = Planner::new_from_supergraph(&parsed_supergraph_sdl)?;
        let metadata = planner.consumer_schema.schema_metadata();
        let authorization = AuthorizationMetadata::build(&planner.supergraph, &metadata)?;
        let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(
            &planner.supergraph.subgraph_endpoint_map,
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
    metrics: Arc<Metrics>,
}

impl SupergraphBackgroundLoader {
    pub fn new(
        config: &SupergraphSource,
        sender: mpsc::Sender<String>,
        metrics: Arc<Metrics>,
    ) -> Result<Self, LoadSupergraphError> {
        let loader = resolve_from_config(config)?;

        Ok(Self {
            loader,
            sender: Arc::new(sender),
            metrics,
        })
    }
}

pub struct SupergraphBackgroundLoaderTask(pub Arc<SupergraphBackgroundLoader>);

#[async_trait]
impl BackgroundTask for SupergraphBackgroundLoaderTask {
    fn id(&self) -> &str {
        "supergraph-background-loader"
    }

    async fn run(&self, token: CancellationToken) {
        let supergraph_metrics = &self.0.metrics.supergraph;
        loop {
            if token.is_cancelled() {
                trace!("Background task cancelled");

                break;
            }

            let poll_capture = supergraph_metrics.capture_poll();
            match self.0.loader.load().await {
                Ok(ReloadSupergraphResult::Unchanged) => {
                    debug!("Supergraph fetched successfully with no changes");
                    poll_capture.finish_not_modified();
                }
                Ok(ReloadSupergraphResult::Changed { new_sdl }) => {
                    debug!("Supergraph loaded successfully with changes, updating...");

                    if self.0.sender.clone().send(new_sdl).await.is_err() {
                        error!("Failed to send new supergraph SDL: receiver dropped.");
                        poll_capture.finish_error();
                        break;
                    }

                    poll_capture.finish_updated();
                }
                Err(err) => {
                    error!("Failed to load supergraph: {}", err);
                    poll_capture.finish_error();
                }
            }

            if let Some(interval) = self.0.loader.reload_interval() {
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
