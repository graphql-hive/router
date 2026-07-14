use crate::pipeline::active_subscriptions::ActiveSubscriptions;
use crate::pipeline::authorization::metadata::AuthorizationMetadataExt;
use crate::storage::StorageManager;
use arc_swap::{ArcSwap, Guard};
use async_trait::async_trait;
use dashmap::DashMap;
use graphql_tools::static_graphql::schema::Document;
use graphql_tools::validation::utils::ValidationError;
use hive_router_config::{supergraph::SupergraphSource, HiveRouterConfig};
use hive_router_internal::telemetry::{metrics::Metrics, TelemetryContext};
use hive_router_internal::{
    authorization::metadata::AuthorizationMetadata,
    background_tasks::{BackgroundTask, BackgroundTasksManager},
};
use hive_router_plan_executor::execution::operation_name::OperationNameForwardConfig;
use hive_router_plan_executor::executors::http_callback::{
    CallbackMessage, CallbackSubscriptionsMap,
};
use hive_router_plan_executor::response::graphql_error::GraphQLErrorExtensions;
use hive_router_plan_executor::{
    executors::error::SubgraphExecutorError,
    hooks::on_supergraph_load::{
        OnSupergraphLoadEndHookPayload, OnSupergraphLoadStartHookPayload, PublicSchema,
        SupergraphData,
    },
    introspection::schema::SchemaWithMetadata,
    plugin_trait::{EndControlFlow, RouterPluginBoxed, StartControlFlow},
    response::graphql_error::GraphQLError,
    SubgraphExecutorMap,
};
use hive_router_query_planner::{
    planner::{plan_nodes::QueryPlan, Planner, PlannerError, QueryPlannerOptions},
    utils::parsing::safe_parse_schema,
};
use moka::future::Cache;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, trace};

use crate::{
    cache_state::CacheState,
    pipeline::{
        authorization::AuthorizationMetadataError, demand_control::runtime::DemandControlRuntime,
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
    pub demand_control_runtime: Option<DemandControlRuntime>,
    pub telemetry_context: Arc<TelemetryContext>,
    pub callback_subscriptions: CallbackSubscriptionsMap,
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

    /// Same as [`Self::from_supergraph_sdl`], but takes an already-parsed supergraph document.
    pub fn from_supergraph_document(
        document: Document,
        router_config: Arc<HiveRouterConfig>,
        telemetry_context: Arc<TelemetryContext>,
    ) -> Result<Self, SupergraphManagerError> {
        let callback_subscriptions: CallbackSubscriptionsMap = Arc::new(DashMap::new());
        let demand_control_runtime = DemandControlRuntime::from_config(
            router_config.demand_control.as_ref(),
            telemetry_context.metrics.clone(),
        );

        let supergraph_data = Self::build_data(
            router_config,
            telemetry_context.clone(),
            document,
            callback_subscriptions.clone(),
        )?;

        Ok(Self {
            current_swapable: Arc::new(ArcSwap::from(Arc::new(Some(supergraph_data)))),
            plan_cache: Cache::new(1000),
            validate_cache: Cache::new(1000),
            normalize_cache: Cache::new(1000),
            demand_control_runtime,
            telemetry_context,
            callback_subscriptions,
        })
    }

    pub async fn new_from_config(
        bg_tasks_manager: &mut BackgroundTasksManager,
        telemetry_context: Arc<TelemetryContext>,
        router_config: Arc<HiveRouterConfig>,
        plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
        cache_state: Arc<CacheState>,
        active_subscriptions: ActiveSubscriptions,
        storage_manager: Arc<StorageManager>,
    ) -> Result<Self, SupergraphManagerError> {
        let (tx, mut rx) = mpsc::channel::<String>(1);
        let background_loader = SupergraphBackgroundLoader::new(
            &router_config.supergraph,
            tx,
            telemetry_context.metrics.clone(),
            storage_manager.clone(),
        )?;
        bg_tasks_manager.register_task(SupergraphBackgroundLoaderTask(Arc::new(background_loader)));

        let swappable_data = Arc::new(ArcSwap::from(Arc::new(None)));
        let swappable_data_spawn_clone = swappable_data.clone();
        let plan_cache = cache_state.plan_cache.clone();
        let validate_cache = cache_state.validate_cache.clone();
        let normalize_cache = cache_state.normalize_cache.clone();
        let callback_subscriptions: CallbackSubscriptionsMap = Arc::new(DashMap::new());

        let demand_control_runtime = DemandControlRuntime::from_config(
            router_config.demand_control.as_ref(),
            telemetry_context.metrics.clone(),
        );

        let demand_control_formula_cache_for_invalidation = demand_control_runtime
            .as_ref()
            .map(|runtime| runtime.formula_cache().clone());

        let cache_state_for_invalidation = cache_state.clone();
        let callback_subscriptions_for_build_data = callback_subscriptions.clone();

        // kick off subscriptions/subgraphs that are idling/timed out due to missed heartbeats
        if let Some(ref callback_config) = router_config.subscriptions.callback {
            if !callback_config.heartbeat_interval.is_zero() {
                let enforcer_subs = callback_subscriptions.clone();
                let heartbeat_interval = callback_config.heartbeat_interval;
                bg_tasks_manager.register_task(CallbackHeartbeatEnforcerTask {
                    callback_subscriptions: enforcer_subs,
                    heartbeat_interval,
                });
            }
        }

        let metrics = telemetry_context.metrics.clone();
        let task_telemetry = telemetry_context.clone();
        let active_subscriptions_for_reload = active_subscriptions.clone();
        bg_tasks_manager.register_handle(async move {
            let supergraph_metrics = &metrics.supergraph;
            while let Some(new_sdl) = rx.recv().await {
                let process_capture = supergraph_metrics.capture_process();
                debug!("Received new supergraph SDL, building new supergraph state...");

                let mut new_ast = match safe_parse_schema(&new_sdl) {
                    Ok(ast) => ast,
                    Err(e) => {
                        process_capture.finish_error();
                        error!(error = %e, "Failed to parse supergraph during update");
                        continue;
                    }
                };

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
                    Self::build_data(
                        router_config.clone(),
                        task_telemetry.clone(),
                        new_ast,
                        callback_subscriptions_for_build_data.clone(),
                    )
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

                        // close all active subscriptions before swapping supergraph data
                        active_subscriptions_for_reload.close_all_with_error(vec![
                            // this is litearaly the same message apollo sends - reasoning is
                            // drop in replacement - is that oke? should we have our own?
                            GraphQLError::from_message_and_code(
                                "subscription has been closed due to a schema reload",
                                "SUBSCRIPTION_SCHEMA_RELOAD",
                            ),
                        ]);

                        swappable_data_spawn_clone.store(Arc::new(Some(new_supergraph_data)));
                        debug!("Supergraph updated successfully");

                        cache_state_for_invalidation.on_schema_change();
                        if let Some(formula_cache) = &demand_control_formula_cache_for_invalidation
                        {
                            formula_cache.invalidate_all();
                        }
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
            demand_control_runtime,
            telemetry_context: telemetry_context.clone(),
            callback_subscriptions,
        })
    }

    fn build_data(
        router_config: Arc<HiveRouterConfig>,
        telemetry_context: Arc<TelemetryContext>,
        parsed_supergraph_sdl: Document,
        callback_subscriptions: CallbackSubscriptionsMap,
    ) -> Result<SupergraphData, SupergraphManagerError> {
        let planner = Planner::new_from_supergraph(
            &parsed_supergraph_sdl,
            QueryPlannerOptions {
                experimental_abstract_type_folding: router_config
                    .query_planner
                    .experimental_abstract_type_folding,
            },
        )?;
        let metadata = Arc::new(planner.consumer_schema.schema_metadata());
        let authorization = AuthorizationMetadata::build(&planner.supergraph, &metadata)?;
        let operation_name_forward_config = Arc::new(OperationNameForwardConfig::new(
            &router_config.traffic_shaping,
            planner.supergraph.known_subgraphs.values(),
        ));
        let subgraph_executor_map = Arc::new(SubgraphExecutorMap::from_http_endpoint_map(
            &planner.supergraph.subgraph_endpoint_map,
            router_config,
            telemetry_context,
            callback_subscriptions,
        )?);

        Ok(SupergraphData {
            supergraph_schema: Arc::new(parsed_supergraph_sdl),
            public_schema: PublicSchema {
                document: planner.consumer_schema.document.clone(),
                sdl: Arc::<str>::from(planner.consumer_schema.document.to_string()),
            },
            metadata,
            planner,
            authorization,
            subgraph_executor_map,
            operation_name_forward_config,
        })
    }
}

/// Router-owned cache mapping a plugin-provided `Arc<Document>` to the `Arc<SchemaState>` built
/// from it. Keyed by allocation identity (`Arc::ptr_eq`), not document content, so plugins must
/// reuse the same `Arc<Document>` per schema variant to get cache hits.
///
/// Strict FIFO eviction: the oldest inserted entry is evicted first once the cache is full,
/// regardless of how often it was hit. Miss construction is serialized by the mutex; this is
/// deliberately simple since schema variants and cache misses are expected to be rare.
pub struct SchemaStateCache {
    entries: Mutex<VecDeque<(Arc<Document>, Arc<SchemaState>)>>,
}

/// Maximum number of distinct schema-document variants kept resolved at once.
const SCHEMA_STATE_CACHE_MAX_SIZE: usize = 10;

impl Default for SchemaStateCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(SCHEMA_STATE_CACHE_MAX_SIZE)),
        }
    }
}

impl SchemaStateCache {
    /// Resolves `document` to its `Arc<SchemaState>`, building and caching it on a miss.
    pub fn resolve(
        &self,
        document: Arc<Document>,
        router_config: Arc<HiveRouterConfig>,
        telemetry_context: Arc<TelemetryContext>,
    ) -> Result<Arc<SchemaState>, SupergraphManagerError> {
        let mut entries = self.entries.lock().unwrap();

        if let Some((_, state)) = entries.iter().find(|(doc, _)| Arc::ptr_eq(doc, &document)) {
            return Ok(state.clone());
        }

        let state = Arc::new(SchemaState::from_supergraph_document(
            document.as_ref().clone(),
            router_config,
            telemetry_context,
        )?);

        if entries.len() >= SCHEMA_STATE_CACHE_MAX_SIZE {
            entries.pop_front();
        }
        entries.push_back((document, state.clone()));

        Ok(state)
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
        storage_manager: Arc<StorageManager>,
    ) -> Result<Self, LoadSupergraphError> {
        let loader = resolve_from_config(config, storage_manager)?;

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

struct CallbackHeartbeatEnforcerTask {
    callback_subscriptions: CallbackSubscriptionsMap,
    heartbeat_interval: Duration,
}

#[async_trait]
impl BackgroundTask for CallbackHeartbeatEnforcerTask {
    fn id(&self) -> &str {
        "http-callback-heartbeat-enforcer"
    }

    async fn run(&self, token: CancellationToken) {
        use std::time::Instant;

        loop {
            tokio::select! {
                _ = token.cancelled() => {
                    debug!("heartbeat enforcer cancelled, stopping");
                    return;
                }
                _ = ntex::time::sleep(self.heartbeat_interval) => {}
            }

            let mut timed_out = Vec::new();
            for entry in self.callback_subscriptions.iter() {
                let last = *entry.value().last_heartbeat.lock().unwrap();
                // heartbeat interval and some grace period to account for potential network delays
                #[cfg(not(feature = "testing"))]
                let grace_period = std::time::Duration::from_millis(1000);
                // when dealing with tests that run in parallel in the CI, we need to increase the
                // grace period to avoid flaky tests due to timing issues with runner under pressure
                #[cfg(feature = "testing")]
                let grace_period = std::time::Duration::from_millis(2000);
                let deadline = self.heartbeat_interval + grace_period;
                let elapsed = match last {
                    // first check hasn't arrived yet, measure from creation time instead
                    None => Instant::now().duration_since(entry.value().created_at),
                    Some(last) => Instant::now().duration_since(last),
                };
                if elapsed > deadline {
                    timed_out.push(entry.key().clone());
                }
            }

            // separate iter so that we dont mess up the slice while looping
            for id in timed_out {
                debug!(
                    subscription_id = %id,
                    "terminating subscription due to http callback subgraph missed heartbeat"
                );
                if let Some((_, sub)) = self.callback_subscriptions.remove(&id) {
                    // we dont care about the result of this send, if it fails it means the client
                    // is already gone or too slow, either way we just terminate the subscription
                    let _ = sub.sender.try_send(CallbackMessage::Complete {
                        errors: Some(vec![GraphQLError::from_message_and_extensions(
                            "Subgraph gone due to heartbeat timeout".to_string(),
                            GraphQLErrorExtensions::new_from_code("SUBGRAPH_GONE"),
                        )]),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod schema_state_cache_tests {
    use super::*;

    const TEST_SUPERGRAPH_SDL: &str =
        include_str!("../../../plugin_examples/replace_schema/supergraph.graphql");

    fn test_document() -> Arc<Document> {
        // subgraph executor construction needs a process-wide rustls crypto provider; installing
        // it repeatedly across tests is a harmless no-op (just logs a warning) but is very useful
        // when running only this test in isolation, so we do it here instead of in a global test setup
        crate::init_rustls_crypto_provider();
        Arc::new(safe_parse_schema(TEST_SUPERGRAPH_SDL).expect("valid test supergraph SDL"))
    }

    fn test_config() -> Arc<HiveRouterConfig> {
        Arc::new(HiveRouterConfig::default())
    }

    fn test_telemetry() -> Arc<TelemetryContext> {
        Arc::new(TelemetryContext::from_propagation_config(
            &Default::default(),
        ))
    }

    #[test]
    fn reusing_same_document_returns_same_state() {
        let cache = SchemaStateCache::default();
        let document = test_document();

        let first = cache
            .resolve(document.clone(), test_config(), test_telemetry())
            .unwrap();
        let second = cache
            .resolve(document, test_config(), test_telemetry())
            .unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.entries.lock().unwrap().len(), 1);
    }

    #[test]
    fn different_allocations_create_separate_entries() {
        let cache = SchemaStateCache::default();

        // Structurally identical documents, but distinct `Arc` allocations.
        let a = test_document();
        let b = test_document();
        assert_eq!(a.as_ref(), b.as_ref());

        let state_a = cache.resolve(a, test_config(), test_telemetry()).unwrap();
        let state_b = cache.resolve(b, test_config(), test_telemetry()).unwrap();

        assert!(!Arc::ptr_eq(&state_a, &state_b));
        assert_eq!(cache.entries.lock().unwrap().len(), 2);
    }

    #[test]
    fn eleventh_unique_document_evicts_the_first() {
        let cache = SchemaStateCache::default();
        let documents: Vec<Arc<Document>> = (0..11).map(|_| test_document()).collect();

        for document in &documents[..10] {
            cache
                .resolve(document.clone(), test_config(), test_telemetry())
                .unwrap();
        }
        assert_eq!(
            cache.entries.lock().unwrap().len(),
            SCHEMA_STATE_CACHE_MAX_SIZE
        );

        cache
            .resolve(documents[10].clone(), test_config(), test_telemetry())
            .unwrap();

        let entries = cache.entries.lock().unwrap();
        assert_eq!(entries.len(), SCHEMA_STATE_CACHE_MAX_SIZE);
        assert!(!entries
            .iter()
            .any(|(doc, _)| Arc::ptr_eq(doc, &documents[0])));
        assert!(entries
            .iter()
            .any(|(doc, _)| Arc::ptr_eq(doc, &documents[10])));
    }

    #[test]
    fn cache_hits_do_not_refresh_fifo_order() {
        let cache = SchemaStateCache::default();
        let first = test_document();
        let second = test_document();

        cache
            .resolve(first.clone(), test_config(), test_telemetry())
            .unwrap();
        cache
            .resolve(second, test_config(), test_telemetry())
            .unwrap();

        // hitting the first (oldest) entry again must not move it to the back.
        cache
            .resolve(first.clone(), test_config(), test_telemetry())
            .unwrap();

        let entries = cache.entries.lock().unwrap();
        assert!(Arc::ptr_eq(&entries.front().unwrap().0, &first));
    }

    #[test]
    fn reusing_an_evicted_document_rebuilds_its_state() {
        let cache = SchemaStateCache::default();
        let evicted = test_document();

        cache
            .resolve(evicted.clone(), test_config(), test_telemetry())
            .unwrap();
        let first_state = cache.entries.lock().unwrap().front().unwrap().1.clone();

        // fill the cache with 10 other unique documents so `evicted` falls off the front.
        for _ in 0..10 {
            cache
                .resolve(test_document(), test_config(), test_telemetry())
                .unwrap();
        }
        assert!(!cache
            .entries
            .lock()
            .unwrap()
            .iter()
            .any(|(doc, _)| Arc::ptr_eq(doc, &evicted)));

        let rebuilt_state = cache
            .resolve(evicted, test_config(), test_telemetry())
            .unwrap();
        assert!(!Arc::ptr_eq(&first_state, &rebuilt_state));
    }

    #[test]
    fn failed_build_does_not_evict_a_valid_cached_state() {
        let cache = SchemaStateCache::default();
        let valid = test_document();
        cache
            .resolve(valid.clone(), test_config(), test_telemetry())
            .unwrap();

        // a malformed subgraph endpoint URL fails executor construction gracefully (unlike a
        // structurally-broken document, which would fail on the safe_parse_schema step
        let invalid_sdl =
            TEST_SUPERGRAPH_SDL.replace("http://0.0.0.0:4200/accounts", "not a valid url ::");
        let invalid = Arc::new(safe_parse_schema(&invalid_sdl).expect("still parses as SDL"));
        let result = cache.resolve(invalid, test_config(), test_telemetry());
        assert!(result.is_err());

        let entries = cache.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(Arc::ptr_eq(&entries.front().unwrap().0, &valid));
    }
}
