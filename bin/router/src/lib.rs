pub mod background_tasks;
mod consts;
mod http_utils;
mod jwt;
mod logger;
pub mod pipeline;
mod schema_state;
mod shared_state;
mod supergraph;
mod utils;

use std::sync::Arc;

use crate::{
    background_tasks::BackgroundTasksManager,
    consts::ROUTER_VERSION,
    http_utils::{
        landing_page::landing_page_handler,
        probes::{health_check_handler, readiness_check_handler},
    },
    jwt::JwtAuthRuntime,
    logger::configure_logging,
    pipeline::{graphql_request_handler, usage_reporting::init_hive_user_agent},
};

pub use crate::{schema_state::SchemaState, shared_state::RouterSharedState};

use hive_router_config::{load_config, HiveRouterConfig};
use http::header::RETRY_AFTER;
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};
use tracing::{info, warn};

async fn graphql_endpoint_handler(
    request: HttpRequest,
    body_bytes: Bytes,
    schema_state: web::types::State<Arc<SchemaState>>,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> impl web::Responder {
    let maybe_supergraph = schema_state.current_supergraph();

    if let Some(supergraph) = maybe_supergraph.as_ref() {
        // If an early CORS response is needed, return it immediately.
        if let Some(early_response) = app_state
            .cors_runtime
            .as_ref()
            .and_then(|cors| cors.get_early_response(&request))
        {
            return early_response;
        }

        let mut res = graphql_request_handler(
            &request,
            body_bytes,
            supergraph,
            app_state.get_ref(),
            schema_state.get_ref(),
        )
        .await;

        // Apply CORS headers to the final response if CORS is configured.
        if let Some(cors) = app_state.cors_runtime.as_ref() {
            cors.set_headers(&request, res.headers_mut());
        }

        res
    } else {
        warn!("No supergraph available yet, unable to process request");

        web::HttpResponse::ServiceUnavailable()
            .header(RETRY_AFTER, 10)
            .finish()
    }
}

pub async fn router_entrypoint() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::var("ROUTER_CONFIG_FILE_PATH").ok();
    let router_config = load_config(config_path)?;
    configure_logging(&router_config.log);
    info!("hive-router@{} starting...", ROUTER_VERSION);
    let http_config = router_config.http.clone();
    let addr = router_config.http.address();
    let mut bg_tasks_manager = BackgroundTasksManager::new();
    let (shared_state, schema_state) =
        configure_app_from_config(router_config, &mut bg_tasks_manager).await?;

    let maybe_error = web::HttpServer::new(move || {
        let lp_gql_path = http_config.graphql_endpoint().to_string();
        web::App::new()
            .state(shared_state.clone())
            .state(schema_state.clone())
            .configure(|m| configure_ntex_app(m, http_config.graphql_endpoint()))
            .default_service(web::to(move || landing_page_handler(lp_gql_path.clone())))
    })
    .bind(addr)?
    .run()
    .await
    .map_err(|err| err.into());

    info!("server stopped, clearning background tasks");
    bg_tasks_manager.shutdown();

    maybe_error
}

pub async fn configure_app_from_config(
    router_config: HiveRouterConfig,
    bg_tasks_manager: &mut BackgroundTasksManager,
) -> Result<(Arc<RouterSharedState>, Arc<SchemaState>), Box<dyn std::error::Error>> {
    let jwt_runtime = match router_config.jwt.is_jwt_auth_enabled() {
        true => Some(JwtAuthRuntime::init(bg_tasks_manager, &router_config.jwt).await?),
        false => None,
    };

    let hive_usage_agent = match router_config.usage_reporting.enabled {
        true => Some(init_hive_user_agent(
            bg_tasks_manager,
            &router_config.usage_reporting,
        )?),
        false => None,
    };

    let router_config_arc = Arc::new(router_config);
    let schema_state =
        SchemaState::new_from_config(bg_tasks_manager, router_config_arc.clone()).await?;
    let schema_state_arc = Arc::new(schema_state);
    let shared_state = Arc::new(RouterSharedState::new(
        router_config_arc,
        jwt_runtime,
        hive_usage_agent,
    )?);

    Ok((shared_state, schema_state_arc))
}

pub fn configure_ntex_app(cfg: &mut web::ServiceConfig, graphql_path: &str) {
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
