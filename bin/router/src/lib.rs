mod cache_state;
mod consts;
pub mod error;
mod http_utils;
mod jwt;
pub mod pipeline;
pub mod plugins;
mod schema_state;
mod shared_state;
mod supergraph;
pub mod telemetry;
mod utils;

use std::sync::Arc;

use crate::{
    consts::ROUTER_VERSION,
    error::RouterInitError,
    http_utils::{
        landing_page::landing_page_handler,
        probes::{health_check_handler, readiness_check_handler},
    },
    jwt::JwtAuthRuntime,
    pipeline::{
        error::handle_pipeline_error,
        graphql_request_handler,
        header::ResponseMode,
        request_extensions::{
            read_graphql_operation_metric_identity, read_graphql_response_metric_status,
            read_request_body_size, write_graphql_response_metric_status,
        },
        timeout::handle_timeout,
        usage_reporting::init_hive_usage_agent,
        validation::{
            max_aliases_rule::MaxAliasesRule, max_depth_rule::MaxDepthRule,
            max_directives_rule::MaxDirectivesRule,
        },
    },
    plugins::plugins_service::PluginService,
    telemetry::{HeaderExtractor, PrometheusAttached},
};

use crate::cache_state::{register_cache_size_observers, CacheState};
pub use crate::plugins::registry::PluginRegistry;
pub use crate::{schema_state::SchemaState, shared_state::RouterSharedState};
pub use arc_swap::ArcSwap;
pub use async_trait::async_trait;
pub use dashmap::DashMap;
pub use graphql_tools;
use graphql_tools::validation::rules::default_rules_validation_plan;
pub use hive_router_config::humantime_serde;
use hive_router_config::{load_config, HiveRouterConfig};
pub use hive_router_internal::background_tasks;
use hive_router_internal::telemetry::metrics::catalog::values::GraphQLResponseStatus;
use hive_router_internal::telemetry::{
    otel::tracing_opentelemetry::OpenTelemetrySpanExt,
    traces::spans::http_request::HttpServerRequestSpan, TelemetryContext,
};
pub use hive_router_internal::BoxError;
pub use hive_router_plan_executor::execution::plan::PlanExecutionOutput;
pub use hive_router_plan_executor::executors::http::SubgraphHttpResponse;
pub use hive_router_plan_executor::response::graphql_error::GraphQLError;
pub use hive_router_query_planner as query_planner;
pub use http;
pub use mimalloc::MiMalloc as RouterGlobalAllocator;
pub use ntex;
pub use ntex::main;
use ntex::web::{self, HttpRequest};
pub use sonic_rs;
pub use tokio;
pub use tracing;
use tracing::{info, warn, Instrument};

static LABORATORY_HTML: &str = include_str!(concat!(env!("OUT_DIR"), "/laboratory.html"));

async fn graphql_endpoint_handler(
    request: HttpRequest,
    body_stream: web::types::Payload,
    schema_state: web::types::State<Arc<SchemaState>>,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> web::HttpResponse {
    let http_request_capture = app_state
        .telemetry_context
        .metrics
        .http_server
        .capture_request(&request);

    let response =
        graphql_endpoint_dispatch(&request, body_stream, schema_state, app_state.clone()).await;

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
    request: &HttpRequest,
    body_stream: web::types::Payload,
    schema_state: web::types::State<Arc<SchemaState>>,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> web::HttpResponse {
    let parent_ctx = app_state
        .telemetry_context
        .extract_context(&HeaderExtractor(request.headers()));
    let root_http_request_span = HttpServerRequestSpan::from_request(request);
    let _ = root_http_request_span.set_parent(parent_ctx);

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
        );

        // Handle the request with a timeout. If the timeout is reached, a timeout error response will be generated.
        let result = handle_timeout(req_handler_fut, &app_state).await;
        let mut response = match result {
            Ok(response) => response,
            // If the request handler returns an error, convert it to an HTTP response.
            Err(err) => {
                write_graphql_response_metric_status(request, GraphQLResponseStatus::Error);
                handle_pipeline_error(err, &app_state, &response_mode)
            }
        };

        // Apply CORS headers to the final response if CORS is configured.
        if let Some(cors) = app_state.cors_runtime.as_ref() {
            cors.set_headers(request, response.headers_mut());
        }

        root_http_request_span.record_response(&response);

        response
    }
    .instrument(root_http_request_span.clone())
    .await
}

pub async fn router_entrypoint(plugin_registry: PluginRegistry) -> Result<(), RouterInitError> {
    let config_path = std::env::var("ROUTER_CONFIG_FILE_PATH").ok();
    let router_config = load_config(config_path)?;
    let telemetry = telemetry::Telemetry::init_global(&router_config)?;
    let prometheus = telemetry
        .prometheus
        .as_ref()
        .and_then(|prom| prom.to_attached());
    info!("hive-router@{} starting...", ROUTER_VERSION);
    let http_config = router_config.http.clone();
    let addr = router_config.http.address();
    let mut bg_tasks_manager = background_tasks::BackgroundTasksManager::new();
    let (shared_state, schema_state) = configure_app_from_config(
        router_config,
        telemetry.context.clone(),
        &mut bg_tasks_manager,
        plugin_registry,
    )
    .await?;

    let shared_state_clone = shared_state.clone();
    let graphql_path = http_config.graphql_endpoint();

    let paths = RouterPaths::new(graphql_path.to_string());
    paths.detect_conflicts(&prometheus)?;

    let graphql_path = graphql_path.to_string();
    let maybe_error = web::HttpServer::new(async move || {
        let landing_page_path = graphql_path.clone();
        let prometheus = prometheus.clone();
        web::App::new()
            .middleware(PluginService)
            .state(shared_state.clone())
            .state(schema_state.clone())
            .configure(|m| configure_ntex_app(m, &paths, prometheus))
            .default_service(web::to(move || {
                landing_page_handler(landing_page_path.clone())
            }))
    })
    .bind(&addr)
    .map_err(|err| RouterInitError::HttpServerBindError(addr, err))?
    .run()
    .await
    .map_err(RouterInitError::HttpServerStartError);

    info!("server stopped, clearing background tasks");
    bg_tasks_manager.shutdown();
    telemetry.graceful_shutdown().await;

    invoke_shutdown_hooks(&shared_state_clone).await;

    maybe_error
}

pub async fn invoke_shutdown_hooks(shared_state: &RouterSharedState) {
    if let Some(plugins) = &shared_state.plugins {
        info!("invoking plugin shutdown hooks");
        for plugin in plugins.as_ref() {
            plugin.on_shutdown().await;
        }
    }
}

pub async fn configure_app_from_config(
    router_config: HiveRouterConfig,
    telemetry_context: TelemetryContext,
    bg_tasks_manager: &mut background_tasks::BackgroundTasksManager,
    plugin_registry: PluginRegistry,
) -> Result<(Arc<RouterSharedState>, Arc<SchemaState>), RouterInitError> {
    let jwt_runtime = match router_config.jwt.is_jwt_auth_enabled() {
        true => Some(JwtAuthRuntime::init(bg_tasks_manager, &router_config.jwt).await?),
        false => None,
    };

    let hive_usage_agent = match router_config.telemetry.hive.as_ref() {
        Some(hive_config) if hive_config.usage_reporting.enabled => {
            Some(init_hive_usage_agent(bg_tasks_manager, hive_config)?)
        }
        _ => None,
    };
    let plugins_arc = plugin_registry.initialize_plugins(&router_config, bg_tasks_manager)?;

    let router_config_arc = Arc::new(router_config);
    let telemetry_context_arc = Arc::new(telemetry_context);
    let cache_state = Arc::new(CacheState::new());

    if router_config_arc.telemetry.metrics.is_enabled() {
        register_cache_size_observers(telemetry_context_arc.clone(), cache_state.clone());
    }

    let schema_state = SchemaState::new_from_config(
        bg_tasks_manager,
        telemetry_context_arc.clone(),
        router_config_arc.clone(),
        plugins_arc.clone(),
        cache_state.clone(),
    )
    .await?;
    let schema_state_arc = Arc::new(schema_state);
    let mut validation_plan = default_rules_validation_plan();
    if let Some(max_depth_config) = &router_config_arc.limits.max_depth {
        validation_plan.add_rule(Box::new(MaxDepthRule {
            config: max_depth_config.clone(),
        }));
    }
    if let Some(max_directives_config) = &router_config_arc.limits.max_directives {
        validation_plan.add_rule(Box::new(MaxDirectivesRule {
            config: max_directives_config.clone(),
        }));
    }
    if let Some(max_aliases_config) = &router_config_arc.limits.max_aliases {
        validation_plan.add_rule(Box::new(MaxAliasesRule {
            config: max_aliases_config.clone(),
        }));
    }
    let shared_state = Arc::new(RouterSharedState::new(
        router_config_arc,
        jwt_runtime,
        hive_usage_agent,
        validation_plan,
        telemetry_context_arc,
        plugins_arc,
        cache_state,
    )?);

    Ok((shared_state, schema_state_arc))
}

#[derive(Clone)]
pub struct RouterPaths {
    graphql: String,
    health: String,
    readiness: String,
}

impl RouterPaths {
    pub fn new(graphql: String) -> Self {
        RouterPaths {
            graphql,
            health: "/health".to_string(),
            readiness: "/readiness".to_string(),
        }
    }

    pub fn detect_conflicts(
        &self,
        prometheus: &Option<PrometheusAttached>,
    ) -> Result<(), RouterInitError> {
        // A pair of context and actual path
        let mut paths = vec![
            ("graphql", self.graphql.as_str()),
            ("health", self.health.as_str()),
            ("readiness", self.readiness.as_str()),
        ];

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

pub fn configure_ntex_app(
    cfg: &mut web::ServiceConfig,
    paths: &RouterPaths,
    prometheus: Option<PrometheusAttached>,
) {
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
        warn!("Rustls crypto provider already installed");
    }
}

#[macro_export]
macro_rules! configure_global_allocator {
    () => {
        #[global_allocator]
        static GLOBAL: RouterGlobalAllocator = RouterGlobalAllocator;
    };
}
