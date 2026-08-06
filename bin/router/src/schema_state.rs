use crate::pipeline::active_subscriptions::ActiveSubscriptions;
use crate::storage::StorageManager;
use arc_swap::ArcSwap;
use async_trait::async_trait;
use dashmap::DashMap;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use graphql_tools::validation::utils::ValidationError;
use hive_console_sdk::agent::usage_agent::UsageAgent;
use hive_router_config::telemetry::hive::{
    is_slug_target_ref, is_uuid_target_ref, HiveTelemetryConfig,
};
use hive_router_config::{supergraph::SupergraphSource, HiveRouterConfig};
use hive_router_internal::authorization::metadata::AuthorizationMetadata;
use hive_router_internal::background_tasks::{
    BackgroundTask, BackgroundTasksManager, DynamicBackgroundTaskRegistrar,
};
use hive_router_internal::telemetry::logging::targets;
use hive_router_internal::telemetry::utils::resolve_value_or_expression;
use hive_router_internal::telemetry::{metrics::Metrics, TelemetryContext};
use hive_router_plan_executor::execution::operation_name::OperationNameForwardConfig;
use hive_router_plan_executor::executors::http_callback::{
    CallbackMessage, CallbackSubscriptionsMap,
};
use hive_router_plan_executor::response::graphql_error::GraphQLErrorExtensions;
use hive_router_plan_executor::{
    execution::error_masking::ErrorMaskingRuntime,
    executors::{error::SubgraphExecutorError, map::HttpCallbackRuntimeConfig},
    hooks::on_supergraph_load::{
        OnSupergraphLoadEndHookPayload, OnSupergraphLoadStartHookPayload, Supergraph,
        SupergraphBuildError, SupergraphOptions, SupergraphSnapshot,
    },
    plugin_trait::{EndControlFlow, RouterPluginBoxed, StartControlFlow},
    response::graphql_error::GraphQLError,
    SubgraphExecutorMap,
};
use hive_router_query_planner::{
    planner::plan_nodes::QueryPlan, utils::parsing::safe_parse_schema,
};
use http::Uri;
use moka::future::Cache;
use ntex::web::HttpRequest;
use std::collections::hash_map;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

use hive_router_plan_executor::headers::{
    compile::compile_headers_plan, errors::HeaderRuleCompileError, plan::HeaderRulesPlan,
};

use crate::{
    pipeline::authorization::AuthorizationMetadataError,
    pipeline::authorization::AuthorizationMetadataExt,
    pipeline::demand_control::runtime::DemandControlRuntime,
    pipeline::normalize::GraphQLNormalizationPayload,
    pipeline::persisted_documents::{
        resolve::PersistedDocumentResolverError, PersistedDocumentsRuntime,
    },
    pipeline::progressive_override::{OverrideLabelsCompileError, OverrideLabelsEvaluator},
    pipeline::usage_reporting::{init_hive_usage_agent, UsageReportingError},
    supergraph::{
        base::{LoadSupergraphError, ReloadSupergraphResult, SupergraphLoader},
        resolve_from_config,
    },
};

#[derive(Debug, thiserror::Error)]
pub enum RouterSupergraphRuntimeError {
    #[error(transparent)]
    ExecutorInitError(#[from] SubgraphExecutorError),
    #[error(transparent)]
    AuthorizationMetadataError(#[from] AuthorizationMetadataError),
    #[error(transparent)]
    HeaderRuleCompileError(#[from] HeaderRuleCompileError),
    #[error(transparent)]
    OverrideLabelsCompileError(#[from] OverrideLabelsCompileError),
    #[error("Invalid router callback configuration: {0}")]
    CallbackConfiguration(String),
    #[error(transparent)]
    UsageReportingError(#[from] UsageReportingError),
    #[error(transparent)]
    PersistedDocumentsError(#[from] PersistedDocumentResolverError),
    #[error("Persisted-document selectors are incompatible with GraphQL endpoint '{0}'")]
    PersistedDocumentsEndpoint(String),
    #[error("Invalid Hive Tracing target format: '{0}'. It must be either in slug format '$organizationSlug/$projectSlug/$targetSlug' or UUID format 'a0f4c605-6541-4350-8cfe-b31f21a4bf80'")]
    HiveTarget(String),
}

/// Router state derived from a supergraph and router configuration: subgraph executors,
/// operation-name forwarding, authorization metadata, and schema-aware caches. The planner,
/// public schema, and plugin-relevant schema metadata live in [`SupergraphSnapshot`] instead.
///
/// This type retains neither the [`Supergraph`] owner nor its snapshot. Requests and streams carry
/// the snapshot separately for schema access and retirement checks.
///
/// Authorization metadata is schema-derived in what it consumes, but it is only ever read by the
/// router's authorization pipeline (which also depends on router configuration), and plugins
/// constructing a [`Supergraph`] never need it. It is therefore built here with the rest of the
/// router runtime rather than in the executor crate.
///
/// The runtime also holds the schema-dependent caches for validation, normalization, planning,
/// and demand-control formulas. Evicting the runtime drops those caches once active users release it.
pub struct RouterSupergraphRuntime {
    pub subgraph_executor_map: Arc<SubgraphExecutorMap>,
    pub operation_name_forward_config: Arc<OperationNameForwardConfig>,
    pub headers_plan: Arc<HeaderRulesPlan>,
    pub override_labels_evaluator: OverrideLabelsEvaluator,
    pub error_masking: Arc<Option<ErrorMaskingRuntime>>,
    pub hive_usage_agent: Option<UsageAgent>,
    pub persisted_documents: PersistedDocumentsRuntime,
    pub authorization: AuthorizationMetadata,
    pub validate_cache: Cache<u64, Arc<Vec<ValidationError>>>,
    pub normalize_cache: Cache<u64, Arc<GraphQLNormalizationPayload>>,
    pub plan_cache: Cache<u64, Arc<QueryPlan>>,
    pub demand_control_runtime: Option<DemandControlRuntime>,
    lifetime: CancellationToken,
}

fn warn_unknown_subgraphs<'a>(
    setting: &str,
    known: &HashSet<&str>,
    names: impl IntoIterator<Item = &'a String>,
) {
    for name in names {
        if !known.contains(name.as_str()) {
            warn!(
                target: targets::SUPERGRAPH,
                setting,
                subgraph = name,
                "configuration refers to a subgraph absent from the selected supergraph"
            );
        }
    }
}

impl RouterSupergraphRuntime {
    pub async fn build(
        snapshot: &SupergraphSnapshot,
        context: &RouterSupergraphRuntimeContext,
    ) -> Result<Self, RouterSupergraphRuntimeError> {
        let known_subgraphs: HashSet<_> = snapshot
            .planner
            .supergraph
            .subgraph_endpoint_map
            .keys()
            .map(String::as_str)
            .collect();
        warn_unknown_subgraphs(
            "traffic_shaping.subgraphs",
            &known_subgraphs,
            snapshot.options.traffic_shaping.subgraphs.keys(),
        );
        warn_unknown_subgraphs(
            "override_subgraph_urls.subgraphs",
            &known_subgraphs,
            snapshot.options.override_subgraph_urls.subgraphs.keys(),
        );
        if let Some(headers) = snapshot.options.headers.subgraphs.as_ref() {
            warn_unknown_subgraphs("headers.subgraphs", &known_subgraphs, headers.keys());
        }
        if let Some(demand_control) = snapshot.options.demand_control.as_ref() {
            if let Some(subgraphs) = demand_control.default_list_size.subgraphs.as_ref() {
                warn_unknown_subgraphs(
                    "demand_control.default_list_size.subgraphs",
                    &known_subgraphs,
                    subgraphs.keys(),
                );
            }
            if let Some(subgraphs) = demand_control.subgraphs_budget.subgraphs.as_ref() {
                warn_unknown_subgraphs(
                    "demand_control.subgraphs_budget.subgraphs",
                    &known_subgraphs,
                    subgraphs.keys(),
                );
            }
        }
        warn_unknown_subgraphs(
            "subscriptions.callback.subgraphs",
            &known_subgraphs,
            snapshot.options.subscriptions.callback_subgraphs.iter(),
        );
        if let Some(websocket) = snapshot.options.subscriptions.websocket.as_ref() {
            warn_unknown_subgraphs(
                "subscriptions.websocket.subgraphs",
                &known_subgraphs,
                websocket.subgraphs.keys(),
            );
        }
        if let Some(error_masking) = snapshot.options.error_masking.subgraphs.as_ref() {
            warn_unknown_subgraphs(
                "error_masking.subgraphs",
                &known_subgraphs,
                error_masking.keys(),
            );
        }

        let subgraph_executor_map = Arc::new(SubgraphExecutorMap::from_http_endpoint_map(
            &snapshot.planner.supergraph.subgraph_endpoint_map,
            snapshot.options.traffic_shaping.clone(),
            snapshot.options.override_subgraph_urls.clone(),
            snapshot.options.subscriptions.clone(),
            context.callback.clone(),
            context.telemetry.clone(),
            context.callback_subscriptions.clone(),
        )?);
        let operation_name_forward_config = Arc::new(OperationNameForwardConfig::new(
            &snapshot.options.traffic_shaping,
            snapshot.planner.supergraph.known_subgraphs.values(),
        ));
        let authorization =
            AuthorizationMetadata::build(&snapshot.planner.supergraph, &snapshot.metadata)?;
        let demand_control_runtime = DemandControlRuntime::from_config(
            snapshot.options.demand_control.as_ref(),
            context.telemetry.metrics.clone(),
        );
        if let Some(target) = snapshot.options.hive_target.as_deref() {
            if !is_uuid_target_ref(target) && !is_slug_target_ref(target) {
                return Err(RouterSupergraphRuntimeError::HiveTarget(target.to_string()));
            }
        } else if context
            .hive
            .as_ref()
            .is_some_and(|hive| hive.tracing.enabled)
        {
            return Err(RouterSupergraphRuntimeError::HiveTarget(
                "Hive tracing is enabled but no target was provided".to_string(),
            ));
        }
        let lifetime = CancellationToken::new();
        let maybe_runtime: Result<Self, RouterSupergraphRuntimeError> = async {
            let persisted_documents = PersistedDocumentsRuntime::init(
                &snapshot.options.persisted_documents,
                &context.graphql_endpoint,
                &context.task_registrar,
                lifetime.clone(),
                &context.storage_manager,
            )
            .await?;
            if !persisted_documents.supports_graphql_endpoint(&context.graphql_endpoint) {
                return Err(RouterSupergraphRuntimeError::PersistedDocumentsEndpoint(
                    context.graphql_endpoint.clone(),
                ));
            }
            let hive_usage_agent = context
                .hive
                .as_ref()
                .filter(|hive| hive.usage_reporting.enabled)
                .map(|hive| {
                    init_hive_usage_agent(
                        &context.task_registrar,
                        lifetime.clone(),
                        hive,
                        snapshot.options.hive_target.as_deref(),
                    )
                })
                .transpose()?;
            Ok(Self {
                subgraph_executor_map,
                operation_name_forward_config,
                headers_plan: Arc::new(compile_headers_plan(&snapshot.options.headers)?),
                override_labels_evaluator: OverrideLabelsEvaluator::from_config(
                    &snapshot.options.override_labels,
                )?,
                error_masking: Arc::new(ErrorMaskingRuntime::compile_from_config(
                    &snapshot.options.error_masking,
                )),
                hive_usage_agent,
                persisted_documents,
                authorization,
                validate_cache: Cache::new(1000),
                normalize_cache: Cache::new(1000),
                plan_cache: Cache::new(1000),
                demand_control_runtime,
                lifetime: lifetime.clone(),
            })
        }
        .await;
        if maybe_runtime.is_err() {
            // cancel everything because we failed to build the runtime
            lifetime.cancel();
        }
        maybe_runtime
    }
}

impl Drop for RouterSupergraphRuntime {
    fn drop(&mut self) {
        self.lifetime.cancel();
    }
}

pub struct RouterSupergraphRuntimeContext {
    telemetry: Arc<TelemetryContext>,
    callback_subscriptions: CallbackSubscriptionsMap,
    callback: Option<HttpCallbackRuntimeConfig>,
    task_registrar: DynamicBackgroundTaskRegistrar,
    hive: Option<HiveTelemetryConfig>,
    storage_manager: Arc<StorageManager>,
    graphql_endpoint: String,
}

/// One selected supergraph for a request: the schema snapshot plus the router runtime built for
/// it. Either resolved from a plugin-selected snapshot (lazily, via the runtime cache) or from
/// the router's configured default (built eagerly before publication).
#[derive(Clone)]
pub struct SelectedSupergraph {
    pub snapshot: SupergraphSnapshot,
    pub runtime: Arc<RouterSupergraphRuntime>,
}

/// The current configured supergraph (from the router config): the owner handle (kept alive only
/// by this slot and whatever schema-load hooks may be holding onto during a reload), its snapshot,
/// and its eagerly built runtime, published together as one atomic value so a request can never
/// observe a mismatched generation.
struct ConfiguredSupergraph {
    // retained only so it stays alive while it is the current configured value. dropping it
    // on the next successful reload publishes retirement, terminates its subscriptions, and
    // lets the runtime and its caches be cleaned up once in-flight users release them
    _owner: Arc<Supergraph>,
    snapshot: SupergraphSnapshot,
    runtime: Arc<RouterSupergraphRuntime>,
}

impl From<&ConfiguredSupergraph> for SelectedSupergraph {
    fn from(configured: &ConfiguredSupergraph) -> Self {
        SelectedSupergraph {
            snapshot: configured.snapshot.clone(),
            runtime: configured.runtime.clone(),
        }
    }
}

const RUNTIME_CACHE_MAX_SIZE: usize = 10;

type RuntimeCell = tokio::sync::OnceCell<Arc<RouterSupergraphRuntime>>;
type RouterSupergraphRuntimeCache = Mutex<VecDeque<(u64, Arc<RuntimeCell>)>>;

pub struct SchemaState {
    /// The supergraph configured through the router config that can be loaded (and polled)
    ///   - `Some` when the router's configured supergraph is available and has been loaded
    ///   - sometimes `None` when the supergraph is being fetched and built
    ///   - always `None` when the router is configured with `supergraph.source: plugin`
    configured: Arc<ArcSwap<Option<ConfiguredSupergraph>>>,
    // the cache of `RouterSupergraphRuntime`s for selected supergraphs, bounded by FIFO eviction
    runtime_cache: Arc<RouterSupergraphRuntimeCache>,
    // sender half for `RuntimeCacheCleanupTask` - registers a cache entry's retirement token so
    // the cleanup task removes it from the cache once its owner retires, and notifies it of FIFO
    // evictions so it can drop the now-pointless waiter instead of leaving it dormant. `None`
    // when the runtime cache cleanup task hasn't been registered (e.g. in tests constructing
    // `SchemaState` directly).
    runtime_cache_cleanup: Option<mpsc::UnboundedSender<RuntimeCacheCleanupMessage>>,
    pub telemetry_context: Arc<TelemetryContext>,
    pub callback_subscriptions: CallbackSubscriptionsMap,
    runtime_context: Arc<RouterSupergraphRuntimeContext>,
}

#[derive(Debug, thiserror::Error)]
pub enum SupergraphManagerError {
    #[error("Failed to load supergraph: {0}")]
    LoadSupergraphError(#[from] LoadSupergraphError),

    #[error(transparent)]
    SupergraphDataBuildError(#[from] SupergraphBuildError),
    #[error(transparent)]
    RouterSupergraphRuntimeError(#[from] RouterSupergraphRuntimeError),

    #[error("Unexpected: failed to load initial supergraph")]
    FailedToLoadInitialSupergraph,

    #[error("Error from plugin: {0}")]
    PluginError(String),

    #[error("Invalid supergraph configuration: {0}")]
    Configuration(String),
}

fn supergraph_options(
    config: &HiveRouterConfig,
) -> Result<SupergraphOptions, SupergraphManagerError> {
    let hive_target = config
        .telemetry
        .hive
        .as_ref()
        .and_then(|hive| hive.target.as_ref())
        .map(|target| {
            resolve_value_or_expression(target, "Hive Telemetry target")
                .map_err(|error| SupergraphManagerError::Configuration(error.to_string()))
        })
        .transpose()?;

    Ok(SupergraphOptions {
        query_planner: hive_router_query_planner::planner::QueryPlannerOptions {
            experimental_abstract_type_folding: config
                .query_planner
                .experimental_abstract_type_folding,
        },
        traffic_shaping: (&config.traffic_shaping).into(),
        override_subgraph_urls: config.override_subgraph_urls.clone(),
        headers: config.headers.clone(),
        override_labels: config.override_labels.clone(),
        demand_control: config.demand_control.clone(),
        subscriptions: (&config.subscriptions).into(),
        error_masking: config.error_masking.clone(),
        persisted_documents: config.persisted_documents.clone(),
        hive_target,
    })
}

// TODO: clean up errors, maybe use lib/executor/src/executors/error.rs#Callback* errors
fn callback_runtime_config(
    config: &HiveRouterConfig,
) -> Result<Option<HttpCallbackRuntimeConfig>, SupergraphManagerError> {
    let Some(callback) = config.subscriptions.callback.as_ref() else {
        return Ok(None);
    };
    let raw = resolve_value_or_expression(&callback.public_url, "subscription callback public URL")
        .map_err(|error| SupergraphManagerError::Configuration(error.to_string()))?;
    let public_url = raw.parse::<Uri>().map_err(|error| {
        SupergraphManagerError::Configuration(format!(
            "invalid callback public URL '{raw}': {error}"
        ))
    })?;
    if public_url.scheme().is_none() || public_url.authority().is_none() {
        return Err(SupergraphManagerError::Configuration(format!(
            "callback public URL must be absolute: '{raw}'"
        )));
    }
    Ok(Some(HttpCallbackRuntimeConfig {
        public_url,
        heartbeat_interval: callback.heartbeat_interval,
    }))
}

impl ConfiguredSupergraph {
    async fn build(
        new_sdl: String,
        current: &ArcSwap<Option<ConfiguredSupergraph>>,
        router_config: &HiveRouterConfig,
        plugins: Option<&Arc<Vec<RouterPluginBoxed>>>,
        runtime_context: &RouterSupergraphRuntimeContext,
    ) -> Result<Self, SupergraphManagerError> {
        let mut new_ast = safe_parse_schema(&new_sdl).map_err(SupergraphBuildError::from)?;
        let mut on_end_callbacks = vec![];
        let mut new_supergraph = None;

        if let Some(plugins) = plugins {
            let current_supergraph_data = current
                .load()
                .as_ref()
                .as_ref()
                .map(SelectedSupergraph::from)
                .map(|selected| selected.snapshot);
            let mut start_payload = OnSupergraphLoadStartHookPayload {
                current_supergraph_data,
                new_ast,
            };
            for plugin in plugins.as_ref() {
                let result = plugin.on_supergraph_reload(start_payload);
                start_payload = result.payload;
                match result.control_flow {
                    StartControlFlow::Proceed => {}
                    StartControlFlow::EndWithResponse(plugin_res) => {
                        new_supergraph = Some(
                            plugin_res
                                .map_err(|err| SupergraphManagerError::PluginError(err.message)),
                        );
                        break;
                    }
                    StartControlFlow::OnEnd(callback) => on_end_callbacks.push(callback),
                }
            }
            new_ast = start_payload.new_ast;
        }

        let options = supergraph_options(router_config)?;
        let mut new_supergraph = new_supergraph.unwrap_or_else(|| {
            Supergraph::from_document(new_ast, options).map_err(SupergraphManagerError::from)
        })?;

        if !on_end_callbacks.is_empty() {
            let mut end_payload = OnSupergraphLoadEndHookPayload { new_supergraph };
            for callback in on_end_callbacks {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    EndControlFlow::Proceed => {}
                    EndControlFlow::EndWithResponse(plugin_res) => match plugin_res {
                        Ok(data) => end_payload.new_supergraph = data,
                        Err(err) => return Err(SupergraphManagerError::PluginError(err.message)),
                    },
                }
            }
            new_supergraph = end_payload.new_supergraph;
        }

        let snapshot = new_supergraph.snapshot();
        let runtime = RouterSupergraphRuntime::build(&snapshot, runtime_context).await?;
        Ok(Self {
            _owner: Arc::new(new_supergraph),
            snapshot,
            runtime: Arc::new(runtime),
        })
    }
}

impl SchemaState {
    /// Resolves the supergraph for a request, preferring a plugin-selected supergraph if present,
    /// falling back to the router's configured default if not. Returns `None` if neither is present.
    pub async fn select_supergraph(
        &self,
        req: &HttpRequest,
    ) -> Result<Option<SelectedSupergraph>, RouterSupergraphRuntimeError> {
        // already selected for this request (by a plugin or by the router's configured default)
        let already_selected = req.extensions().get::<SelectedSupergraph>().cloned();
        if let Some(already_selected) = already_selected {
            return Ok(Some(already_selected));
        }

        // not selected yet, maybe a plugin selected one for this request - this must be checked
        // before the configured default, otherwise a plugin's override could never take effect

        let plugin_supergraph = req.extensions().get::<SupergraphSnapshot>().cloned();
        if let Some(plugin_supergraph) = plugin_supergraph {
            // a plugin selected a supergraph for this request, maybe we cached its runtime (fast)
            // and if we didnt cache the runtime, we will build a new one (slow)

            let runtime = self.resolve_runtime(&plugin_supergraph).await?;

            let selected = SelectedSupergraph {
                snapshot: plugin_supergraph,
                runtime,
            };

            debug!(target: targets::SUPERGRAPH, internal_id = selected.snapshot.cache_id, "supergraph was set from a plugin");

            req.extensions_mut().insert(selected.clone());

            return Ok(Some(selected));
        }

        // no plugin-selected supergraph, fall back to the router's configured default
        let selected = self
            .configured
            .load()
            .as_ref()
            .as_ref()
            .map(SelectedSupergraph::from);

        if let Some(selected) = &selected {
            debug!(target: targets::SUPERGRAPH, "using supergraph from the configured default");

            req.extensions_mut().insert(selected.clone());
        }

        Ok(selected)
    }

    /// Returns the router's currently configured default runtime, if any (`None` for
    /// `supergraph.source: plugin`, or before the first supergraph has loaded).
    pub fn configured_runtime(&self) -> Option<Arc<RouterSupergraphRuntime>> {
        self.configured
            .load()
            .as_ref()
            .as_ref()
            .map(|configured| configured.runtime.clone())
    }

    /// Calls `f` for every runtime currently alive: every plugin-selected runtime still sitting
    /// in the bounded FIFO cache, plus the configured default (if any).
    pub fn for_each_runtime(&self, mut f: impl FnMut(&RouterSupergraphRuntime)) {
        if let Some(configured) = self.configured.load().as_ref() {
            f(&configured.runtime);
        }
        for (_, runtime) in self.runtime_cache.lock().unwrap().iter() {
            if let Some(runtime) = runtime.get() {
                f(runtime);
            }
        }
    }

    /// Resolves the runtime for a snapshot. The configured runtime is returned directly; plugin
    /// runtimes use the bounded FIFO cache, where cache hits do not refresh FIFO order.
    async fn resolve_runtime(
        &self,
        snapshot: &SupergraphSnapshot,
    ) -> Result<Arc<RouterSupergraphRuntime>, RouterSupergraphRuntimeError> {
        let cache_id = snapshot.cache_id;

        // skip all of the synchronisation mechanisms if the user is requesting the configured supergraph
        let configured_runtime = self
            .configured
            .load()
            .as_ref()
            .as_ref()
            .filter(|configured| configured.snapshot.cache_id == cache_id)
            .map(|configured| configured.runtime.clone());
        if let Some(runtime) = configured_runtime {
            return Ok(runtime);
        }

        let (cell, evicted, inserted) = {
            // its ok for the lock to expire in this scope - we only need to check for an existing
            // entry and insert a new one if missing. the runtime itself is built asynchronously
            // and cached in the `OnceCell` so that multiple requests racing to build the same runtime
            // only build it once without sync blocking the router by using a mutex
            let mut entries = self.runtime_cache.lock().unwrap();
            if let Some((_, runtime)) = entries.iter().find(|(id, _)| *id == cache_id) {
                (runtime.clone(), None, false)
            } else {
                let evicted = (entries.len() >= RUNTIME_CACHE_MAX_SIZE)
                    .then(|| entries.pop_front().map(|(id, _)| id))
                    .flatten();
                let cell = Arc::new(RuntimeCell::new());
                entries.push_back((cache_id, cell.clone()));
                (cell, evicted, true)
            }
        };

        if inserted {
            if let Some(sender) = &self.runtime_cache_cleanup {
                if let Some(evicted_id) = evicted {
                    sender
                        .send(RuntimeCacheCleanupMessage::Evicted(evicted_id))
                        .ok();
                }
                sender
                    .send(RuntimeCacheCleanupMessage::Registered(
                        cache_id,
                        snapshot.retirement_token(),
                    ))
                    .ok();
            }
        }

        match cell
            .get_or_try_init(|| async {
                RouterSupergraphRuntime::build(snapshot, &self.runtime_context)
                    .await
                    .map(Arc::new)
            })
            .await
        {
            Ok(runtime) => Ok(runtime.clone()),
            Err(error) => {
                self.runtime_cache
                    .lock()
                    .unwrap()
                    .retain(|(id, entry)| *id != cache_id || !Arc::ptr_eq(entry, &cell));
                Err(error)
            }
        }
    }

    /// Returns true if the router is ready to serve requests, i.e. if a supergraph is available for
    /// the request (either plugin-selected or configured default).
    pub async fn is_ready(&self, req: &HttpRequest) -> bool {
        matches!(self.select_supergraph(req).await, Ok(Some(selected)) if !selected.snapshot.is_retired())
    }

    pub async fn new_from_config(
        bg_tasks_manager: &mut BackgroundTasksManager,
        telemetry_context: Arc<TelemetryContext>,
        router_config: Arc<HiveRouterConfig>,
        plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
        active_subscriptions: ActiveSubscriptions,
        storage_manager: Arc<StorageManager>,
        task_registrar: DynamicBackgroundTaskRegistrar,
    ) -> Result<Self, SupergraphManagerError> {
        let configured: Arc<ArcSwap<Option<ConfiguredSupergraph>>> =
            Arc::new(ArcSwap::from(Arc::new(None)));

        // single callback-subscriptions map for the router: the configured reload path and
        // every lazily built plugin runtime wire their subgraph executors to this same map, and
        // the heartbeat enforcer below watches it too. building a runtime with a *different* map
        // would silently break callback routing and heartbeat enforcement for it
        let callback_subscriptions: CallbackSubscriptionsMap = Arc::new(DashMap::new());
        let runtime_context = Arc::new(RouterSupergraphRuntimeContext {
            telemetry: telemetry_context.clone(),
            callback_subscriptions: callback_subscriptions.clone(),
            callback: callback_runtime_config(&router_config)?,
            task_registrar,
            hive: router_config.telemetry.hive.clone(),
            storage_manager: storage_manager.clone(),
            graphql_endpoint: router_config.http.graphql_endpoint.clone(),
        });

        // `supergraph.source: plugin` has no configured source at all... no loader, no polling
        // task, no configured-default value. a plugin must select a supergraph for every request
        // that needs one and the plugin author is responsible for maintaining the supergraphs
        if !matches!(router_config.supergraph, SupergraphSource::Plugin) {
            let (tx, mut rx) = mpsc::channel::<String>(1);
            let background_loader = Arc::new(SupergraphBackgroundLoader::new(
                &router_config.supergraph,
                tx,
                telemetry_context.metrics.clone(),
                storage_manager.clone(),
            )?);

            let initial_sdl = match background_loader.loader.load().await? {
                ReloadSupergraphResult::Changed { new_sdl } => new_sdl,
                ReloadSupergraphResult::Unchanged => {
                    return Err(SupergraphManagerError::FailedToLoadInitialSupergraph)
                }
            };
            let initial = ConfiguredSupergraph::build(
                initial_sdl,
                &configured,
                &router_config,
                plugins.as_ref(),
                &runtime_context,
            )
            .await?;
            configured.store(Arc::new(Some(initial)));

            bg_tasks_manager.register_task(SupergraphBackgroundLoaderTask(background_loader));

            let configured_spawn_clone = configured.clone();
            let router_config_for_task = router_config.clone();
            let task_telemetry = telemetry_context.clone();
            let runtime_context_for_reload = runtime_context.clone();

            bg_tasks_manager.register_handle(async move {
                let supergraph_metrics = &task_telemetry.metrics.supergraph;
                while let Some(new_sdl) = rx.recv().await {
                    let process_capture = supergraph_metrics.capture_process();
                    debug!("Received new supergraph SDL, building new supergraph state...");

                    match ConfiguredSupergraph::build(
                        new_sdl,
                        &configured_spawn_clone,
                        &router_config_for_task,
                        plugins.as_ref(),
                        &runtime_context_for_reload,
                    )
                    .await
                    {
                        Ok(new_configured) => {
                            // swapping in the new value here is enough: the previous
                            // `ConfiguredSupergraph`'s owner `Arc<Supergraph>` is only kept alive
                            // by this slot (ordinary requests only ever hold a snapshot),
                            // so once it's replaced its `Drop` publishes retirement and every
                            // subscription producer selected from it terminates on its own -
                            // no global subscription closure needed here
                            configured_spawn_clone.store(Arc::new(Some(new_configured)));
                            debug!("Supergraph updated successfully");
                            process_capture.finish_ok();
                        }
                        Err(e) => {
                            process_capture.finish_error();
                            error!("Failed to build new supergraph data: {}", e);
                        }
                    }
                }
            });
        }

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

        // `active_subscriptions` is retained by the caller for the lifetime of the router; no
        // per-supergraph subscription index is needed here since each producer terminates
        // itself by observing its own selected supergraph's retirement token.
        let _ = active_subscriptions;

        let runtime_cache: Arc<RouterSupergraphRuntimeCache> =
            Arc::new(Mutex::new(VecDeque::with_capacity(RUNTIME_CACHE_MAX_SIZE)));
        let (cleanup_tx, cleanup_rx) = mpsc::unbounded_channel();
        bg_tasks_manager.register_task(RuntimeCacheCleanupTask {
            runtime_cache: runtime_cache.clone(),
            registrations: tokio::sync::Mutex::new(cleanup_rx),
        });

        Ok(Self {
            configured,
            runtime_cache,
            runtime_cache_cleanup: Some(cleanup_tx),
            telemetry_context: telemetry_context.clone(),
            callback_subscriptions,
            runtime_context,
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
                trace!(target: targets::SUPERGRAPH, "background task cancelled");

                break;
            }

            let poll_capture = supergraph_metrics.capture_poll();
            match self.0.loader.load().await {
                Ok(ReloadSupergraphResult::Unchanged) => {
                    debug!(target: targets::SUPERGRAPH, "supergraph fetched successfully with no changes");
                    poll_capture.finish_not_modified();
                }
                Ok(ReloadSupergraphResult::Changed { new_sdl }) => {
                    info!(target: targets::SUPERGRAPH, "supergraph loaded successfully with changes, updating...");

                    if self.0.sender.clone().send(new_sdl).await.is_err() {
                        error!(target: targets::SUPERGRAPH, "failed to send new supergraph SDL: receiver dropped");
                        poll_capture.finish_error();

                        break;
                    }

                    poll_capture.finish_updated();
                }
                Err(err) => {
                    error!(target: targets::SUPERGRAPH, error = ?err, "failed to load supergraph");
                    poll_capture.finish_error();
                }
            }

            if let Some(interval) = self.0.loader.reload_interval() {
                debug!(target: targets::SUPERGRAPH, interval_ms = interval.as_millis(), "waiting before checking again for supergraph changes");

                ntex::time::sleep(interval).await;
            } else {
                debug!(target: targets::SUPERGRAPH, "poll interval not configured for supergraph changes, skipping");

                break;
            }
        }
    }
}

/// Message sent to `RuntimeCacheCleanupTask` over its registration channel.
enum RuntimeCacheCleanupMessage {
    /// A new cache entry was inserted - watch its retirement token so the entry can be removed
    /// promptly once its owner retires, instead of waiting for FIFO eviction.
    Registered(u64, CancellationToken),
    /// A cache entry was pushed out by bounded FIFO eviction - if a waiter was registered for
    /// it, drop it. Otherwise it would keep watching a retirement token for an entry that's
    /// already gone from the cache, sitting dormant until (if ever) the owner retires.
    Evicted(u64),
}

/// Router-managed background task that removes runtime-cache entries once their owner retires,
/// instead of waiting for bounded FIFO eviction to eventually push them out.
///
/// Runtimes are registered dynamically (one registration per cache insertion, after router
/// initialization), so this is a single long-lived task fed through `registrations` rather than
/// an unmanaged spawned task per runtime. Registration is deduplicated by cache id: a FIFO
/// eviction followed by reinsertion of the same still-live owner must not create a second waiter
/// for it.
///
/// This does not make the runtime cache itself reject retired entries - it only ever removes
/// them eventually. Requests already holding a cloned `Arc<RouterSupergraphRuntime>` are
/// unaffected by the removal; see the retirement checks in `pipeline/mod.rs` and
/// `pipeline/websocket_server.rs` that still gate on retirement directly.
struct RuntimeCacheCleanupTask {
    runtime_cache: Arc<RouterSupergraphRuntimeCache>,
    registrations: tokio::sync::Mutex<mpsc::UnboundedReceiver<RuntimeCacheCleanupMessage>>,
}

#[async_trait]
impl BackgroundTask for RuntimeCacheCleanupTask {
    fn id(&self) -> &str {
        "runtime-cache-cleanup"
    }

    async fn run(&self, token: CancellationToken) {
        let mut registrations = self.registrations.lock().await;

        // per-waiter cancellation, so an evicted entry's waiter can be dropped without waiting
        // for its retirement token (which may never fire, or fire long after eviction)
        let mut waiter_cancels: HashMap<u64, CancellationToken> = HashMap::new();
        let mut waiters = FuturesUnordered::new();

        loop {
            tokio::select! {
                _ = token.cancelled() => {
                    debug!("runtime cache cleanup task cancelled, stopping");
                    return;
                }
                registered = registrations.recv() => {
                    match registered {
                        None => {
                            debug!("runtime cache cleanup registration channel closed, stopping");
                            return;
                        }
                        Some(RuntimeCacheCleanupMessage::Registered(cache_id, retirement)) => {
                            // dedup by cache id: FIFO eviction + reinsertion of the same live
                            // owner must not create a second waiter for it
                            if let hash_map::Entry::Vacant(entry) =
                                waiter_cancels.entry(cache_id)
                            {
                                let waiter_cancel = CancellationToken::new();
                                entry.insert(waiter_cancel.clone());
                                waiters.push(async move {
                                    tokio::select! {
                                        _ = retirement.cancelled() => Some(cache_id),
                                        _ = waiter_cancel.cancelled() => None,
                                    }
                                });
                            }
                        }
                        Some(RuntimeCacheCleanupMessage::Evicted(cache_id)) => {
                            // cancel the waiter for this id (if any) - it would otherwise sit
                            // dormant, watching a retirement token for an entry that's already
                            // gone from the cache
                            if let Some(waiter_cancel) = waiter_cancels.remove(&cache_id) {
                                waiter_cancel.cancel();
                            }
                        }
                    }
                }
                Some(resolved) = waiters.next(), if !waiters.is_empty() => {
                    let Some(cache_id) = resolved else {
                        // waiter was cancelled due to eviction, already removed from
                        // `waiter_cancels` at cancellation time
                        continue;
                    };
                    waiter_cancels.remove(&cache_id);
                    let mut entries = self.runtime_cache.lock().unwrap();
                    // no-op if already gone (FIFO eviction raced us, or already removed) -
                    // removing the entry only drops the cache's Arc, active requests and
                    // streams hold their own clone and keep running through it
                    entries.retain(|(id, _)| *id != cache_id);
                }
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
                    debug!(target: targets::HTTP_CALLBACK, "heartbeat enforcer cancelled, stopping");
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
                    target: targets::HTTP_CALLBACK,
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
mod plugin_runtime_cache_tests {
    use super::*;

    const TEST_SUPERGRAPH_SDL: &str =
        include_str!("../../../plugin_examples/replace_schema/supergraph.graphql");

    fn test_schema_state() -> SchemaState {
        let telemetry_context = Arc::new(TelemetryContext::from_propagation_config(
            &Default::default(),
            &Default::default(),
        ));
        let callback_subscriptions = Arc::new(DashMap::new());
        let runtime_context = Arc::new(RouterSupergraphRuntimeContext {
            telemetry: telemetry_context.clone(),
            callback_subscriptions: callback_subscriptions.clone(),
            callback: None,
            task_registrar: BackgroundTasksManager::new().dynamic_registrar(),
            hive: None,
            storage_manager: Arc::new(StorageManager::new(&Default::default()).unwrap()),
            graphql_endpoint: "/graphql".to_string(),
        });
        SchemaState {
            configured: Arc::new(ArcSwap::from(Arc::new(None))),
            runtime_cache: Arc::new(Mutex::new(VecDeque::with_capacity(RUNTIME_CACHE_MAX_SIZE))),
            runtime_cache_cleanup: None,
            telemetry_context,
            callback_subscriptions,
            runtime_context,
        }
    }

    fn test_owner() -> Arc<Supergraph> {
        crate::init_rustls_crypto_provider();
        Arc::new(
            Supergraph::from_sdl(TEST_SUPERGRAPH_SDL, SupergraphOptions::default())
                .expect("valid test supergraph SDL"),
        )
    }

    #[ntex::test]
    async fn graph_bound_options_stay_isolated_between_live_owners() {
        let state = test_schema_state();
        let mut first_options = SupergraphOptions::default();
        first_options.hive_target = Some("example/router/first".to_string());
        first_options.error_masking.redacted_error_message = "first".to_string();
        let mut second_options = SupergraphOptions::default();
        second_options.hive_target = Some("example/router/second".to_string());
        second_options.error_masking.redacted_error_message = "second".to_string();
        second_options.traffic_shaping.all.forward_operation_name = true;

        let first = Arc::new(Supergraph::from_sdl(TEST_SUPERGRAPH_SDL, first_options).unwrap());
        let second = Arc::new(Supergraph::from_sdl(TEST_SUPERGRAPH_SDL, second_options).unwrap());
        let first_runtime = state.resolve_runtime(&first.snapshot()).await.unwrap();
        let second_runtime = state.resolve_runtime(&second.snapshot()).await.unwrap();

        assert_eq!(
            first.options.hive_target.as_deref(),
            Some("example/router/first")
        );
        assert_eq!(
            second.options.hive_target.as_deref(),
            Some("example/router/second")
        );
        assert_eq!(first.options.error_masking.redacted_error_message, "first");
        assert_eq!(
            second.options.error_masking.redacted_error_message,
            "second"
        );
        assert!(!first_runtime
            .operation_name_forward_config
            .should_forward("products"));
        assert!(second_runtime
            .operation_name_forward_config
            .should_forward("products"));
        assert!(!Arc::ptr_eq(&first_runtime, &second_runtime));
    }

    #[ntex::test]
    async fn reusing_same_supergraph_reuses_one_runtime() {
        let state = test_schema_state();
        let owner = test_owner();
        let snapshot = owner.snapshot();

        let first = state.resolve_runtime(&snapshot).await.unwrap();
        let second = state.resolve_runtime(&snapshot).await.unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(state.runtime_cache.lock().unwrap().len(), 1);
    }

    #[ntex::test]
    async fn concurrent_resolves_build_one_runtime() {
        let state = Arc::new(test_schema_state());
        let owner = test_owner();
        let snapshot = owner.snapshot();
        let runtimes = futures::future::join_all((0..8).map(|_| {
            let state = state.clone();
            let snapshot = snapshot.clone();
            async move { state.resolve_runtime(&snapshot).await.unwrap() }
        }))
        .await;

        // the first thread builds while holding the mutex, then every other thread observes
        // its cache entry instead of building and inserting another runtime
        assert!(runtimes[1..]
            .iter()
            .all(|runtime| Arc::ptr_eq(&runtimes[0], runtime)));
        assert_eq!(state.runtime_cache.lock().unwrap().len(), 1);
    }

    #[ntex::test]
    async fn distinct_supergraph_instances_get_distinct_runtimes() {
        let state = test_schema_state();

        let a = test_owner();
        let b = test_owner();
        // distinct cache ids even though the content is identical.
        assert_ne!(a.cache_id, b.cache_id);

        let runtime_a = state.resolve_runtime(&a.snapshot()).await.unwrap();
        let runtime_b = state.resolve_runtime(&b.snapshot()).await.unwrap();

        assert!(!Arc::ptr_eq(&runtime_a, &runtime_b));
        assert_eq!(state.runtime_cache.lock().unwrap().len(), 2);
    }

    #[ntex::test]
    async fn configured_selection_stays_pinned_to_the_request_after_rotation() {
        let state = test_schema_state();
        let first_owner = test_owner();
        let first_snapshot = first_owner.snapshot();
        let first_id = first_snapshot.cache_id;
        let first_runtime = Arc::new(
            RouterSupergraphRuntime::build(&first_snapshot, &state.runtime_context)
                .await
                .unwrap(),
        );
        state.configured.store(Arc::new(Some(ConfiguredSupergraph {
            _owner: first_owner,
            snapshot: first_snapshot.clone(),
            runtime: first_runtime.clone(),
        })));

        let resolved_runtime = state.resolve_runtime(&first_snapshot).await.unwrap();
        assert!(Arc::ptr_eq(&resolved_runtime, &first_runtime));
        assert!(state.runtime_cache.lock().unwrap().is_empty());

        let req = ntex::web::test::TestRequest::default().to_http_request();
        assert_eq!(
            state
                .select_supergraph(&req)
                .await
                .unwrap()
                .unwrap()
                .snapshot
                .cache_id,
            first_id
        );

        let second_owner = test_owner();
        let second_snapshot = second_owner.snapshot();
        let second_runtime = Arc::new(
            RouterSupergraphRuntime::build(&second_snapshot, &state.runtime_context)
                .await
                .unwrap(),
        );
        state.configured.store(Arc::new(Some(ConfiguredSupergraph {
            _owner: second_owner,
            snapshot: second_snapshot,
            runtime: second_runtime,
        })));

        let selected = state.select_supergraph(&req).await.unwrap().unwrap();
        assert_eq!(selected.snapshot.cache_id, first_id);
        assert!(selected.snapshot.is_retired());
    }

    #[ntex::test]
    async fn eleventh_unique_supergraph_evicts_the_first() {
        let state = test_schema_state();
        let owners: Vec<Arc<Supergraph>> = (0..11).map(|_| test_owner()).collect();

        for owner in &owners[..10] {
            state.resolve_runtime(&owner.snapshot()).await.unwrap();
        }
        assert_eq!(
            state.runtime_cache.lock().unwrap().len(),
            RUNTIME_CACHE_MAX_SIZE
        );

        state.resolve_runtime(&owners[10].snapshot()).await.unwrap();

        let entries = state.runtime_cache.lock().unwrap();
        assert_eq!(entries.len(), RUNTIME_CACHE_MAX_SIZE);
        assert!(!entries.iter().any(|(id, _)| *id == owners[0].cache_id));
        assert!(entries.iter().any(|(id, _)| *id == owners[10].cache_id));
    }

    #[ntex::test]
    async fn cache_hits_do_not_refresh_fifo_order() {
        let state = test_schema_state();
        let first = test_owner();
        let second = test_owner();

        state.resolve_runtime(&first.snapshot()).await.unwrap();
        state.resolve_runtime(&second.snapshot()).await.unwrap();
        state.resolve_runtime(&first.snapshot()).await.unwrap();

        let entries = state.runtime_cache.lock().unwrap();
        assert_eq!(entries.front().unwrap().0, first.cache_id);
    }

    #[ntex::test]
    async fn dropping_owner_marks_snapshot_retired_without_a_cleanup_task() {
        let state = test_schema_state();
        let owner = test_owner();
        let snapshot = owner.snapshot();

        state.resolve_runtime(&snapshot).await.unwrap();

        drop(owner);

        // this isolated state deliberately has no cleanup task, so retirement is observable
        // while the bounded cache entry remains until FIFO eviction
        assert!(snapshot.is_retired());
        assert_eq!(state.runtime_cache.lock().unwrap().len(), 1);
    }

    #[ntex::test]
    async fn cleanup_task_removes_cache_entry_once_owner_retires() {
        let mut state = test_schema_state();
        let runtime_cache = state.runtime_cache.clone();
        let (cleanup_tx, cleanup_rx) = mpsc::unbounded_channel();
        state.runtime_cache_cleanup = Some(cleanup_tx);

        let task = RuntimeCacheCleanupTask {
            runtime_cache: runtime_cache.clone(),
            registrations: tokio::sync::Mutex::new(cleanup_rx),
        };
        let cancel = CancellationToken::new();
        let task_handle = ntex::rt::spawn({
            let cancel = cancel.clone();
            async move { task.run(cancel).await }
        });

        let owner = test_owner();
        let snapshot = owner.snapshot();
        state.resolve_runtime(&snapshot).await.unwrap();
        assert_eq!(runtime_cache.lock().unwrap().len(), 1);

        drop(owner);

        // cleanup runs asynchronously - poll briefly instead of assuming an immediate removal
        for _ in 0..100 {
            if runtime_cache.lock().unwrap().is_empty() {
                break;
            }
            ntex::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(runtime_cache.lock().unwrap().is_empty());

        cancel.cancel();
        let _ = task_handle.await;
    }

    /// an `Evicted` message must drop the stale waiter for that id, so that if the same id
    /// is registered again later (e.g. FIFO evicted then the owner retires far later), the
    /// new registration gets its own live waiter instead of being silently swallowed by
    /// leftover dedup state from the evicted registration
    #[ntex::test]
    async fn evicted_message_lets_the_same_id_be_registered_again() {
        let owner = test_owner();
        let cache_id = owner.cache_id;
        let runtime = Arc::new(
            RouterSupergraphRuntime::build(
                &owner.snapshot(),
                &RouterSupergraphRuntimeContext {
                    telemetry: Arc::new(TelemetryContext::from_propagation_config(
                        &Default::default(),
                        &Default::default(),
                    )),
                    callback_subscriptions: Arc::new(DashMap::new()),
                    callback: None,
                    task_registrar: BackgroundTasksManager::new().dynamic_registrar(),
                    hive: None,
                    storage_manager: Arc::new(StorageManager::new(&Default::default()).unwrap()),
                    graphql_endpoint: "/graphql".to_string(),
                },
            )
            .await
            .unwrap(),
        );
        let cell = Arc::new(RuntimeCell::new());
        assert!(cell.set(runtime).is_ok());
        let runtime_cache: Arc<RouterSupergraphRuntimeCache> =
            Arc::new(Mutex::new(VecDeque::from([(cache_id, cell)])));

        let (cleanup_tx, cleanup_rx) = mpsc::unbounded_channel();
        let task = RuntimeCacheCleanupTask {
            runtime_cache: runtime_cache.clone(),
            registrations: tokio::sync::Mutex::new(cleanup_rx),
        };
        let cancel = CancellationToken::new();
        let task_handle = ntex::rt::spawn({
            let cancel = cancel.clone();
            async move { task.run(cancel).await }
        });

        // register a watcher for `cache_id` whose token never fires, then evict it - the fix
        // must drop this waiter rather than leave it dormant
        let stale_retirement = CancellationToken::new();
        cleanup_tx
            .send(RuntimeCacheCleanupMessage::Registered(
                cache_id,
                stale_retirement.clone(),
            ))
            .unwrap();
        cleanup_tx
            .send(RuntimeCacheCleanupMessage::Evicted(cache_id))
            .unwrap();

        // register the same id again (as if it were reinserted later) with a token that *does*
        // fire - if the stale waiter's dedup entry wasn't cleared by eviction, this registration
        // would be silently dropped as a "duplicate" and the entry would never be removed
        let live_retirement = CancellationToken::new();
        cleanup_tx
            .send(RuntimeCacheCleanupMessage::Registered(
                cache_id,
                live_retirement.clone(),
            ))
            .unwrap();

        live_retirement.cancel();

        for _ in 0..100 {
            if runtime_cache.lock().unwrap().is_empty() {
                break;
            }
            ntex::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            runtime_cache.lock().unwrap().is_empty(),
            "entry was not removed - the re-registration was likely swallowed as a stale duplicate"
        );

        cancel.cancel();
        let _ = task_handle.await;
    }
}
