// Needed because of ntex's way of defining middlewares
#![recursion_limit = "256"]

pub mod background_tasks;
mod cache_state;
pub mod config;
mod consts;
pub mod error;
pub mod executor;
mod http_utils;
mod jwt;
pub mod pipeline;
pub mod plugins;
pub mod query_planner;
mod schema_state;
mod shared_state;
mod storage;
mod supergraph;
pub mod telemetry;
mod utils;

use http::{
    header::{ACCEPT, CONTENT_TYPE, USER_AGENT},
    HeaderName,
};
use ntex_http::body::{BodySize, MessageBody};
use std::ops::ControlFlow;
use std::sync::Arc;
use tracing::{debug, error};

use crate::{
    consts::ROUTER_VERSION,
    error::RouterInitError,
    http_utils::{
        body::read_request_body_size,
        landing_page::landing_page_handler,
        probes::{health_check_handler, readiness_check_handler},
    },
    jwt::JwtAuthRuntime,
    pipeline::{
        active_subscriptions::ActiveSubscriptions,
        error::handle_pipeline_error,
        graphql_request_handler,
        header::ResponseMode,
        http_callback::handler,
        long_lived_client_limit::LongLivedClientLimitService,
        persisted_documents::PersistedDocumentsBackgroundTasks,
        request_extensions::{
            read_graphql_operation_metric_identity, read_graphql_response_metric_status,
            write_graphql_response_metric_status,
        },
        request_identifiers::RequestIdentifiersService,
        request_summary::RequestSummaryService,
        timeout::handle_timeout,
        usage_reporting::HiveUsageReportingBackgroundTasks,
        validation::{
            max_aliases_rule::MaxAliasesRule, max_depth_rule::MaxDepthRule,
            max_directives_rule::MaxDirectivesRule,
        },
        websocket_server::ws_index,
    },
    plugins::plugins_service::PluginService,
    storage::StorageManager,
    telemetry::{HeaderExtractor, PrometheusAttached},
};

use crate::background_tasks::{BackgroundTask, CancellationToken};
use crate::cache_state::register_cache_size_observers;
pub use crate::config::humantime_serde;
use crate::config::{load_config, subscriptions::CallbackConfig, HiveRouterConfig};
pub use crate::executor::execution::plan::PlanExecutionOutput;
pub use crate::executor::executors::http::SubgraphHttpResponse;
use crate::executor::headers::response::ResponseHeaderSink;
pub use crate::executor::response::graphql_error::GraphQLError;
pub use crate::pipeline::authorization::user_auth_context::{
    AuthorizationMetadataError, UserAuthContext,
};
pub use crate::plugins::registry::PluginRegistry;
use crate::telemetry::logging::request_id::{self, REQUEST_IDENTIFIERS};
use crate::telemetry::metrics::catalog::values::GraphQLResponseStatus;
use crate::telemetry::{
    logging::{summary, targets},
    traces::spans::http_request::HttpServerRequestSpan,
    utils::RequestRoutePattern,
    TelemetryContext,
};
pub use crate::utils::BoxError;
pub use crate::{schema_state::SchemaState, shared_state::RouterSharedState};
pub use arc_swap::ArcSwap;
pub use async_trait::async_trait;
pub use dashmap::DashMap;
pub use graphql_tools;
use graphql_tools::validation::rules::default_rules_validation_plan;
pub use http;
pub use mimalloc::MiMalloc as RouterGlobalAllocator;
pub use ntex;
pub use ntex::main;
use ntex::{
    http::{HttpServiceConfig, KeepAlive},
    time::Seconds,
    web::{self, HttpRequest},
    SharedCfg,
};
pub use opentelemetry;
pub use sonic_rs;
pub use tokio;
pub use tracing;
use tracing::{info, warn, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;
pub mod tls;
mod vrl;

#[cfg(not(feature = "graphiql"))]
static LABORATORY_HTML: &str = include_str!(concat!(env!("OUT_DIR"), "/laboratory.html"));
#[cfg(feature = "graphiql")]
static LABORATORY_HTML: &str = include_str!("../static/graphiql.html");

struct CallbackServer(std::sync::Mutex<Option<ntex::server::Server>>);

impl From<ntex::server::Server> for CallbackServer {
    fn from(server: ntex::server::Server) -> Self {
        Self(std::sync::Mutex::new(Some(server)))
    }
}

#[async_trait]
impl BackgroundTask for CallbackServer {
    fn id(&self) -> &str {
        "callback_server"
    }

    async fn run(&self, token: CancellationToken) {
        token.cancelled().await;
        // only poisoned if a thread panicked while holding the lock; since the only
        // operation inside is .take(), that can't happen
        let server = self.0.lock().unwrap().take();
        if let Some(server) = server {
            server.stop(true).await;
        }
    }
}

/// Lets plugins enrich the current request's summary log line with a custom attribute.
/// Setting the same key again overwrites the previous value.
///
/// A no-op outside a request (e.g. during `on_plugin_init`) or when the summary log target is filtered off.
pub fn set_summary_attribute(key: impl Into<String>, value: impl Into<sonic_rs::Value>) {
    summary::record(|s| s.set_custom(key, value));
}

/// Returns the current request log summary for the current request, if one exists.
pub fn get_current_summary() -> Option<Arc<summary::RequestSummary>> {
    summary::current_summary()
}

/// Lets plugins attach a custom correlation to every log line of the current request (not
/// just the summary), e.g. a tenant or project id extracted from the URL.
/// Setting the same key again overwrites the previous value. A no-op outside a request.
pub fn set_log_correlation(key: impl Into<String>, value: impl std::fmt::Display) {
    request_id::set_correlation(key, value);
}

/// Lets plugins override the request summary log line's message.
/// This can be called only once per request, and only during the request's lifetime.
/// Calling it more than once, for the same request is a no-op.
/// Calling it outside of a request is a no-op.
pub fn set_summary_message(message: impl Into<std::borrow::Cow<'static, str>>) {
    summary::record(|s| s.set_message(message));
}

#[inline]
fn obtain_header_value<'a>(
    header_map: &'a ntex::http::HeaderMap,
    header_name: &HeaderName,
) -> &'a str {
    header_map
        .get(header_name)
        .map(|h| h.to_str().unwrap_or(""))
        .unwrap_or("")
}

async fn graphql_endpoint_handler(
    mut request: HttpRequest,
    body_stream: web::types::Payload,
    schema_state: web::types::State<Arc<SchemaState>>,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> web::HttpResponse {
    request
        .extensions_mut()
        .insert(RequestRoutePattern(app_state.router_config.graphql_path()));
    let parent_ctx = app_state
        .telemetry_context
        .extract_context(&HeaderExtractor(request.headers()));

    let http_request_capture = app_state
        .telemetry_context
        .metrics
        .http_server
        .capture_request(&request);

    // logged here rather than in `RequestSummaryMiddleware` because we want custom coorelations to be included as well
    debug!(
        target: targets::HTTP_SERVER,
        method = request.method().as_str(),
        path = request.path(),
        query_string = request.query_string(),
        content_type = obtain_header_value(request.headers(), &CONTENT_TYPE),
        accept = obtain_header_value(request.headers(), &ACCEPT),
        user_agent = obtain_header_value(request.headers(), &USER_AGENT),
        "http request started",
    );

    let response = graphql_endpoint_dispatch(
        &mut request,
        body_stream,
        schema_state,
        app_state.clone(),
        parent_ctx,
    )
    .await;

    let status_code = response.status().as_u16();
    let payload_bytes = match response.body().size() {
        BodySize::Empty | BodySize::None => 0,
        BodySize::Sized(size) => i64::try_from(size).unwrap_or(i64::MAX),
        BodySize::Stream => -1,
    };

    debug!(
        target: targets::HTTP_SERVER,
        status_code,
        payload_bytes,
        "http request completed",
    );

    summary::record(|s| {
        s.status_code
            .store(status_code, std::sync::atomic::Ordering::Relaxed);
        s.payload_bytes
            .store(payload_bytes, std::sync::atomic::Ordering::Relaxed);
    });

    let graphql_operation = read_graphql_operation_metric_identity(&request);
    let graphql_operation_name = graphql_operation
        .as_ref()
        .and_then(|operation| operation.operation_name.as_deref());
    let graphql_operation_type = graphql_operation
        .as_ref()
        .and_then(|operation| operation.operation_type);
    let graphql_response_status =
        read_graphql_response_metric_status(&request).unwrap_or(GraphQLResponseStatus::Ok);

    http_request_capture.finish(
        &response,
        read_request_body_size(&request),
        graphql_operation_name,
        graphql_operation_type,
        graphql_response_status,
    );

    response
}

async fn graphql_endpoint_dispatch(
    request: &mut HttpRequest,
    body_stream: web::types::Payload,
    schema_state: web::types::State<Arc<SchemaState>>,
    app_state: web::types::State<Arc<RouterSharedState>>,
    parent_ctx: opentelemetry::Context,
) -> web::HttpResponse {
    let root_http_request_span = HttpServerRequestSpan::from_request(
        request,
        &app_state
            .router_config
            .telemetry
            .client_identification
            .ip_header,
    );
    let _ = root_http_request_span.set_parent(parent_ctx);

    #[allow(unused_must_use)]
    REQUEST_IDENTIFIERS.try_with(|request_identifiers| {
        root_http_request_span.record_request_id(request_identifiers.req_id());
    });

    let response_header_sink = ResponseHeaderSink::default();

    async {
        // Set it to the default value in case of the negotiation failing,
        // so that we can still generate an error response in the correct format.
        // It will be updated to the negotiated value if the negotiation succeeds,
        // inside the graphql_request_handler function.
        let mut response_mode = ResponseMode::default();

        let req_handler_fut = graphql_request_handler(
            request,
            body_stream,
            app_state.get_ref(),
            schema_state.get_ref(),
            &root_http_request_span,
            &mut response_mode,
            response_header_sink.clone(),
        );

        // Handle the request with a timeout. If the timeout is reached, a timeout error response will be generated.
        let result = handle_timeout(req_handler_fut, &app_state).await;
        let mut response = match result {
            Ok(response) => response,
            // If the request handler returns an error, convert it to an HTTP response.
            Err(err) => {
                write_graphql_response_metric_status(request, GraphQLResponseStatus::Error);
                handle_pipeline_error(err, request, &app_state, &response_mode)
            }
        };

        if let Err(err) = response_header_sink
            .take()
            .modify_client_response_headers(response.headers_mut())
        {
            error!(target: targets::HEADER_MANIPULATION, error = %err, "failed to apply response header rules to the outgoing client response");
        }

        // Apply CORS headers to the final response if CORS is configured.
        if let Some(cors) = app_state.cors_runtime.as_ref() {
            cors.set_headers(request, response.headers_mut());
        }

        if let Some(coprocessor_runtime) = app_state.coprocessor.as_ref() {
            response = match coprocessor_runtime
                .on_graphql_response(response, request, || {
                    // reuse the exact snapshot execution already resolved and stored on the
                    // request - never re-resolve here, which could observe a different
                    // generation than the one that actually executed the operation
                    request
                        .extensions()
                        .get::<crate::schema_state::SelectedSupergraph>()
                        .map(|selected| selected.snapshot.public_schema.sdl.clone())
                })
                .await
            {
                Ok(
                    ControlFlow::Break(updated_response) | ControlFlow::Continue(updated_response),
                ) => updated_response,
                Err(error) => {
                    warn!(target: targets::COPROCESSOR, error = ?error, "coprocessor graphql.response stage failed");
                    write_graphql_response_metric_status(request, GraphQLResponseStatus::Error);
                    handle_pipeline_error(error.into(), request, &app_state, &response_mode)
                }
            };
        }

        root_http_request_span.record_response(&response);

        response
    }
    .instrument(root_http_request_span.clone())
    .await
}

/// ntex expresses HTTP/1 keep-alive in whole seconds. Sub-second values round
/// up so a configured timeout cannot collapse to "close immediately", and
/// oversized values clamp instead of wrapping. Zero disables keep-alive.
fn to_ntex_keep_alive(duration: std::time::Duration) -> KeepAlive {
    let secs = duration.as_secs() + u64::from(duration.subsec_nanos() > 0);

    match u16::try_from(secs) {
        Ok(0) => KeepAlive::Disabled,
        Ok(secs) => KeepAlive::Timeout(Seconds(secs)),
        Err(_) => KeepAlive::Timeout(Seconds(u16::MAX)),
    }
}

pub fn build_http_service_config(router_config: &HiveRouterConfig) -> HttpServiceConfig {
    let keep_alive = to_ntex_keep_alive(router_config.keep_alive());

    let ntex_timeout = u16::try_from(
        router_config
            .traffic_shaping
            .router
            .request_timeout
            .as_secs()
            .saturating_add(1),
    )
    .unwrap_or(u16::MAX);

    let max_request_header_size = router_config.limits.max_request_header_size.to_bytes() as usize;

    HttpServiceConfig::new()
        .set_keepalive(keep_alive)
        // ntex HTTP timeout is set as a safe-guard on top of Hive Router's timeout
        .set_client_timeout(Seconds(ntex_timeout))
        // ntex's parse buffer must fit the whole request head, otherwise limits
        // above its 64KiB default would be unreachable; the exact per-request
        // limit is enforced in the pipeline (`graphql_request_handler`)
        .set_max_buf_size(max_request_header_size.max(64 * 1024))
}

pub async fn router_entrypoint(plugin_registry: PluginRegistry) -> Result<(), RouterInitError> {
    if cfg!(debug_assertions) && std::env::var("CARGO").is_err() {
        eprintln!("WARNING: You are running Hive Router using a debug binary, which is not recommended for production use.");
        eprintln!("  Please consider to use the official binary / Docker image instead:");
        eprintln!("    https://the-guild.dev/graphql/hive/docs/router/getting-started");
        eprintln!("  Or, if you are building with custom plugins, refer to the documentation for building from source:");
        eprintln!("    https://the-guild.dev/graphql/hive/docs/router/customizations/plugin-system/usage#build-your-router");
    }

    let config_path = std::env::var("ROUTER_CONFIG_FILE_PATH").ok();
    let router_config = load_config(config_path)?.into_static();
    let telemetry = telemetry::Telemetry::init_global(router_config)?;
    for warning in router_config.from_env_warnings() {
        info!(target: targets::CONFIG, "{warning}");
    }
    let prometheus = telemetry
        .prometheus
        .as_ref()
        .and_then(|prom| prom.to_attached());
    info!(target: targets::CORE, version = ROUTER_VERSION, "hive-router starting...");
    let addr = router_config.address();
    let graphql_path = router_config.graphql_path().to_string();
    let websocket_path = router_config.websocket_path().map(|p| p.to_string());
    let callback_conf = router_config.callback_conf().cloned();
    let workers = router_config.workers();
    let mut bg_tasks_manager = background_tasks::BackgroundTasksManager::new();
    let (shared_state, schema_state) = configure_app_from_config(
        router_config,
        telemetry.context.clone(),
        &mut bg_tasks_manager,
        plugin_registry,
    )
    .await?;

    let shared_state_clone = shared_state.clone();
    let callback_subscriptions_for_handler = schema_state.callback_subscriptions.clone();

    // when `listen` is set, the callback route lives on a dedicated server bound to that address
    // otherwise, the callback route is mounted on the main server on the `callback_path`
    let callback_path = match callback_conf {
        Some(CallbackConfig {
            listen: Some(listen),
            ref path,
            ..
        }) => {
            let cb_path = path.to_string();
            let cb_addr = listen.to_string();
            let cb_subs = callback_subscriptions_for_handler.clone();
            let cb_telemetry_context = shared_state.telemetry_context.clone();
            let mut cb_server_builder = web::HttpServer::new(async move || {
                let cb_subs = cb_subs.clone();
                let cb_path = cb_path.clone();
                let telemetry_context = cb_telemetry_context.clone();
                web::App::new()
                    .state(cb_subs)
                    .state(telemetry_context)
                    .configure(move |m| add_callback_handler(m, &cb_path))
            });
            if let Some(workers) = workers {
                info!(
                    target: targets::CORE,
                    workers_count = workers,
                    "configuring HTTP callback server worker(s)",
                );
                cb_server_builder = cb_server_builder.workers(workers.get());
            }
            let cb_cfg = SharedCfg::new("HIVE_ROUTER_CALLBACK")
                .add(build_http_service_config(router_config));
            let cb_server = cb_server_builder
                .config(cb_cfg)
                .shutdown_timeout(router_config.shutdown_timeout())
                .bind(&cb_addr)
                .map_err(|err| RouterInitError::HttpCallbackServerBindError(cb_addr, err))?
                .run();

            bg_tasks_manager.register_task(CallbackServer::from(cb_server));

            None
        }
        Some(ref cb) => Some(cb.path.to_string()),
        None => None,
    };

    // after callback config check because there we decide if callback_path should be set
    let paths = RouterPaths::new(graphql_path.clone(), websocket_path, callback_path);
    paths.detect_conflicts(&prometheus)?;

    let graphql_path = graphql_path.to_string();
    let long_lived_client_limit_service =
        LongLivedClientLimitService::new(shared_state.router_config);

    let mut server = web::HttpServer::new(async move || {
        let landing_page_path = graphql_path.clone();
        let prometheus = prometheus.clone();
        let long_lived_client_limit_service = long_lived_client_limit_service.clone();
        let paths_for_plugin = paths.clone();
        web::App::new()
            .middleware(long_lived_client_limit_service)
            .middleware(PluginService::new(
                paths_for_plugin,
                prometheus.as_ref().map(|p| p.endpoint.clone()),
            ))
            .middleware(RequestSummaryService::new(&graphql_path))
            .middleware(RequestIdentifiersService)
            .state(shared_state.clone())
            .state(schema_state.clone())
            .state(shared_state.telemetry_context.clone())
            .state(callback_subscriptions_for_handler.clone())
            .configure(|m| configure_ntex_app(m, &paths, prometheus))
            .configure(|m| {
                if let Some(ref callback) = paths.callback {
                    // callback path will be some only if callback is enabled and if
                    // its listen is not configured to be on another server
                    add_callback_handler(m, callback);
                }
            })
            .default_service(web::to(move || {
                landing_page_handler(landing_page_path.clone())
            }))
    });

    if let Some(workers) = workers {
        info!(target: targets::CORE, workers_count = workers, "configuring HTTP server worker(s)");
        server = server.workers(workers.get());
    }

    let cfg = SharedCfg::new("HIVE_ROUTER").add(build_http_service_config(router_config));

    server = server
        .config(cfg)
        .shutdown_timeout(router_config.shutdown_timeout());

    let tls_config = shared_state_clone
        .router_config
        .traffic_shaping
        .router
        .tls
        .as_ref();

    let maybe_error = if let Some(tls_config) = tls_config {
        let rustls_config = tls::build_rustls_config(tls_config)?;
        server.bind_rustls(&addr, &rustls_config)
    } else {
        server.bind(&addr)
    }
    .map_err(|err| RouterInitError::HttpServerBindError(addr.to_string(), err))?
    .run()
    .await
    .map_err(RouterInitError::HttpServerStartError);

    info!(target: targets::CORE, "router stopped, clearing background tasks");
    bg_tasks_manager.graceful_shutdown().await;
    telemetry.graceful_shutdown().await;

    invoke_shutdown_hooks(&shared_state_clone).await;

    maybe_error
}

pub async fn invoke_shutdown_hooks(shared_state: &RouterSharedState) {
    if let Some(plugins) = &shared_state.plugins {
        debug!(target: targets::CORE, "invoking plugin shutdown hooks");

        for plugin in plugins.as_ref() {
            plugin.on_shutdown().await;
        }
    }
}

pub async fn configure_app_from_config(
    router_config: &'static HiveRouterConfig,
    telemetry_context: TelemetryContext,
    bg_tasks_manager: &mut background_tasks::BackgroundTasksManager,
    plugin_registry: PluginRegistry,
) -> Result<(Arc<RouterSharedState>, Arc<SchemaState>), RouterInitError> {
    let jwt_runtime = match router_config.jwt.is_jwt_auth_enabled() {
        true => Some(JwtAuthRuntime::init(bg_tasks_manager, &router_config.jwt).await?),
        false => None,
    };

    let plugins_arc = plugin_registry.initialize_plugins(router_config, bg_tasks_manager)?;

    let active_subscriptions =
        ActiveSubscriptions::new(router_config.subscriptions.broadcast_capacity);
    let storage_manager = Arc::new(StorageManager::new(&router_config.storages)?);
    let telemetry_context_arc = Arc::new(telemetry_context);

    let (persisted_documents_background_tasks, persisted_documents_background_task) =
        PersistedDocumentsBackgroundTasks::new();
    bg_tasks_manager.register_graceful_task(persisted_documents_background_task);
    let (hive_usage_reporting_background_tasks, hive_usage_reporting_background_task) =
        HiveUsageReportingBackgroundTasks::new();
    bg_tasks_manager.register_graceful_task(hive_usage_reporting_background_task);

    let schema_state = SchemaState::new_from_config(
        bg_tasks_manager,
        telemetry_context_arc.clone(),
        router_config,
        plugins_arc.clone(),
        active_subscriptions.clone(),
        storage_manager.clone(),
        persisted_documents_background_tasks,
        hive_usage_reporting_background_tasks,
    )
    .await?;
    let schema_state_arc = Arc::new(schema_state);

    let mut validation_plan = default_rules_validation_plan();
    if let Some(max_depth_config) = &router_config.limits.max_depth {
        validation_plan.add_rule(Box::new(MaxDepthRule {
            config: max_depth_config.clone(),
        }));
    }
    if let Some(max_directives_config) = &router_config.limits.max_directives {
        validation_plan.add_rule(Box::new(MaxDirectivesRule {
            config: max_directives_config.clone(),
        }));
    }
    if let Some(max_aliases_config) = &router_config.limits.max_aliases {
        validation_plan.add_rule(Box::new(MaxAliasesRule {
            config: max_aliases_config.clone(),
        }));
    }
    let metrics_enabled = router_config.telemetry.metrics.is_enabled();
    let shared_state = Arc::new(RouterSharedState::new(
        router_config,
        jwt_runtime,
        validation_plan,
        telemetry_context_arc.clone(),
        plugins_arc,
        active_subscriptions.clone(),
        storage_manager,
    )?);

    if metrics_enabled {
        register_cache_size_observers(
            telemetry_context_arc,
            shared_state.clone(),
            schema_state_arc.clone(),
        );
    }

    Ok((shared_state, schema_state_arc))
}

#[derive(Clone)]
pub struct RouterPaths {
    pub graphql: String,
    websocket: Option<String>,
    callback: Option<String>,
    pub health: String,
    pub readiness: String,
}

impl RouterPaths {
    pub fn new(graphql: String, websocket: Option<String>, callback: Option<String>) -> Self {
        RouterPaths {
            graphql,
            websocket,
            callback,
            health: "/health".to_string(),
            readiness: "/readiness".to_string(),
        }
    }

    pub fn detect_conflicts(
        &self,
        prometheus: &Option<PrometheusAttached>,
    ) -> Result<(), RouterInitError> {
        // A pair of context and actual path (only include optional paths when present)
        let mut paths = vec![
            ("graphql", self.graphql.as_str()),
            ("health", self.health.as_str()),
            ("readiness", self.readiness.as_str()),
        ];

        if let Some(ws) = self.websocket.as_deref() {
            // its safe to have graphql and websocket on same path
            if ws != self.graphql.as_str() {
                paths.push(("websocket", ws));
            }
        }

        if let Some(cb) = self.callback.as_deref() {
            paths.push(("callback", cb));
        }

        if let Some(prom) = prometheus {
            paths.push(("prometheus", prom.endpoint.as_str()));
        }

        for (name_a, path_a) in &paths {
            let conflict = paths
                .iter()
                .find(|(name_b, path_b)| name_a != name_b && path_a == path_b);

            if let Some((name_b, _)) = conflict {
                return Err(RouterInitError::EndpointConflict {
                    endpoint_name_one: (*name_a).to_string(),
                    endpoint_name_two: (*name_b).to_string(),
                    endpoint: (*path_a).to_string(),
                });
            }
        }

        Ok(())
    }
}

pub fn add_callback_handler(cfg: &mut web::ServiceConfig, callback_path: &str) {
    let callback_route = format!(
        "{}/{{subscription_id}}",
        callback_path.trim_end_matches('/'),
    );
    cfg.route(&callback_route, web::post().to(handler));
}

pub fn configure_ntex_app(
    cfg: &mut web::ServiceConfig,
    paths: &RouterPaths,
    prometheus: Option<PrometheusAttached>,
) {
    if let Some(websocket) = &paths.websocket {
        cfg.service(
            web::resource(websocket.as_str())
                // guard ensures this resource is only matched for actual ws upgrade requests,
                // so a plain GET to the same path (e.g. graphql GET request) falls through
                // to the next registered resource instead of hitting the ws handshake
                .guard(web::guard::fn_guard(|head| {
                    head.headers()
                        .get(ntex::http::header::UPGRADE)
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
                }))
                .route(web::get().to(ws_index)),
        );
    }

    cfg.route(paths.graphql.as_str(), web::to(graphql_endpoint_handler))
        .route(paths.health.as_str(), web::to(health_check_handler))
        .route(paths.readiness.as_str(), web::to(readiness_check_handler));

    if let Some(prom) = prometheus {
        let registry = prom.registry;
        cfg.route(
            prom.endpoint.as_str(),
            web::get().to(move || {
                let registry = registry.clone();
                async move { telemetry::build_metrics_response(&registry) }
            }),
        );
    }

    // Enables /graphql/sha256:12345 cases for persisted documents
    if paths.graphql != "/" {
        cfg.service(
            web::scope(paths.graphql.as_str()).default_service(web::to(graphql_endpoint_handler)),
        );
    }
}

/// Initializes the rustls cryptographic provider for the entire process.
///
/// Rustls requires a cryptographic provider to be set as the default before any TLS operations occur.
/// Installs AWS-LC, as `ring` is no longer maintained.
///
/// This function should be called early in the application startup, before any rustls-based TLS
/// connections are established.
/// In the hive-router binary and docker image, it's called automatically during router initialization.
/// This ensures that all TLS operations throughout the application can use the configured provider.
///
/// This function can only be called successfully once per process.
/// Subsequent calls will log a warning, but will not fail.
///
///
/// This allows consumers of the `hive-router` crate to use their own cryptographic provider if needed,
/// by calling this function or setting their own provider before initializing the router.
///
/// This function does not return an error. If the provider is already installed, it logs a warning.
pub fn init_rustls_crypto_provider() {
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        error!(target: targets::TLS, "rustls crypto provider already installed, ignoring");
    }
}

#[macro_export]
macro_rules! configure_global_allocator {
    () => {
        #[global_allocator]
        static GLOBAL: RouterGlobalAllocator = RouterGlobalAllocator;
    };
}

#[cfg(test)]
mod to_ntex_keep_alive_tests {
    use super::to_ntex_keep_alive;
    use ntex::{http::KeepAlive, time::Seconds};
    use std::time::Duration;

    #[test]
    fn keeps_whole_seconds() {
        assert_eq!(
            to_ntex_keep_alive(Duration::from_secs(80)),
            KeepAlive::Timeout(Seconds(80))
        );
    }

    #[test]
    fn rounds_partial_seconds_up() {
        assert_eq!(
            to_ntex_keep_alive(Duration::from_millis(1)),
            KeepAlive::Timeout(Seconds(1))
        );
        assert_eq!(
            to_ntex_keep_alive(Duration::from_millis(1500)),
            KeepAlive::Timeout(Seconds(2))
        );
    }

    #[test]
    fn disables_keep_alive_for_zero() {
        assert_eq!(
            to_ntex_keep_alive(Duration::from_secs(0)),
            KeepAlive::Disabled
        );
    }

    #[test]
    fn clamps_oversized_durations() {
        assert_eq!(
            to_ntex_keep_alive(Duration::from_secs(u64::from(u16::MAX) + 1)),
            KeepAlive::Timeout(Seconds(u16::MAX))
        );
    }
}
