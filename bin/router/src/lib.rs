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
        http_callback::handler,
        timeout::handle_timeout,
        usage_reporting::init_hive_usage_agent,
        validation::{
            max_aliases_rule::MaxAliasesRule, max_depth_rule::MaxDepthRule,
            max_directives_rule::MaxDirectivesRule,
        },
        websocket_server::ws_index,
    },
    plugins::plugins_service::PluginService,
    telemetry::HeaderExtractor,
};

pub use crate::plugins::registry::PluginRegistry;
pub use crate::{schema_state::SchemaState, shared_state::RouterSharedState};
pub use arc_swap::ArcSwap;
pub use async_trait::async_trait;
pub use dashmap::DashMap;
pub use graphql_tools;
use graphql_tools::validation::rules::default_rules_validation_plan;
pub use hive_router_config::humantime_serde;
use hive_router_config::{load_config, subscriptions::CallbackConfig, HiveRouterConfig};
pub use hive_router_internal::background_tasks;
use hive_router_internal::background_tasks::{BackgroundTask, CancellationToken};
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

static GRAPHIQL_HTML: &str = include_str!("../static/graphiql.html");

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

async fn graphql_endpoint_handler(
    request: HttpRequest,
    body_stream: web::types::Payload,
    schema_state: web::types::State<Arc<SchemaState>>,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> web::HttpResponse {
    let parent_ctx = app_state
        .telemetry_context
        .extract_context(&HeaderExtractor(request.headers()));
    let root_http_request_span = HttpServerRequestSpan::from_request(&request);
    let _ = root_http_request_span.set_parent(parent_ctx);

    async {
        // Set it to the default value in case of the negotiation failing,
        // so that we can still generate an error response in the correct format.
        // It will be updated to the negotiated value if the negotiation succeeds,
        // inside the graphql_request_handler function.
        let mut response_mode = ResponseMode::default();

        let req_handler_fut = graphql_request_handler(
            &request,
            body_stream,
            app_state.get_ref(),
            schema_state.get_ref(),
            &root_http_request_span,
            &mut response_mode,
        );

        // Handle the request with a timeout. If the timeout is reached, a timeout error response will be generated.
        let result = handle_timeout(req_handler_fut, &app_state).await;
        let mut res = match result {
            Ok(response) => response,
            // If the request handler returns an error, convert it to an HTTP response.
            Err(error) => handle_pipeline_error(error, &app_state, &response_mode),
        };

        // Apply CORS headers to the final response if CORS is configured.
        if let Some(cors) = app_state.cors_runtime.as_ref() {
            cors.set_headers(&request, res.headers_mut());
        }

        root_http_request_span.record_response(&res);

        res
    }
    .instrument(root_http_request_span.clone())
    .await
}

pub async fn router_entrypoint(plugin_registry: PluginRegistry) -> Result<(), RouterInitError> {
    let config_path = std::env::var("ROUTER_CONFIG_FILE_PATH").ok();
    let router_config = load_config(config_path)?;
    let telemetry = telemetry::Telemetry::init_global(&router_config)?;
    info!("hive-router@{} starting...", ROUTER_VERSION);

    let addr = router_config.address();
    let gql_path = router_config.graphql_path().to_string();
    let websocket_path = router_config.websocket_path().map(|p| p.to_string());
    let callback_conf = router_config.callback_conf().cloned();

    let mut bg_tasks_manager = background_tasks::BackgroundTasksManager::new();
    let (shared_state, schema_state) = configure_app_from_config(
        router_config,
        telemetry.context.clone(),
        &mut bg_tasks_manager,
        plugin_registry,
    )
    .await?;

    let shared_state_clone = shared_state.clone();
    let active_subs = schema_state.active_callback_subscriptions.clone();

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
            let cb_active_subs = active_subs.clone();
            let cb_server = web::HttpServer::new(async move || {
                let cb_active_subs = cb_active_subs.clone();
                let cb_path = cb_path.clone();
                web::App::new()
                    .state(cb_active_subs)
                    .configure(move |m| add_callback_handler(m, &cb_path))
            })
            .bind(&cb_addr)
            .map_err(|err| RouterInitError::HttpCallbackServerBindError(cb_addr, err))?
            .run();

            bg_tasks_manager.register_task(CallbackServer::from(cb_server));

            None
        }
        Some(ref cb) => Some(cb.path.to_string()),
        None => None,
    };

    let maybe_error = web::HttpServer::new(async move || {
        let lp_gql_path = gql_path.clone();
        let active_subs = active_subs.clone();
        let callback_path = callback_path.clone();
        web::App::new()
            .middleware(PluginService)
            .state(shared_state.clone())
            .state(schema_state.clone())
            .state(active_subs)
            .configure(|m| configure_ntex_app(m, gql_path.as_ref(), websocket_path.as_deref()))
            .configure(move |m| {
                if let Some(ref cb_path) = callback_path {
                    // callback_path will be some only if callback is enabled and if
                    // its listen is not configured to be on another server
                    add_callback_handler(m, cb_path);
                }
            })
            .default_service(web::to(move || landing_page_handler(lp_gql_path.clone())))
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
    let schema_state = SchemaState::new_from_config(
        bg_tasks_manager,
        telemetry_context_arc.clone(),
        router_config_arc.clone(),
        plugins_arc.clone(),
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
    )?);

    Ok((shared_state, schema_state_arc))
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
    graphql_path: &str,
    websocket_path: Option<&str>,
) {
    if let Some(websocket_path) = websocket_path {
        cfg.route(websocket_path, web::get().to(ws_index));
    }
    cfg.route(graphql_path, web::to(graphql_endpoint_handler))
        .route("/health", web::to(health_check_handler))
        .route("/readiness", web::to(readiness_check_handler));
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
