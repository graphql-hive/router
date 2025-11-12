pub mod background_tasks;
mod consts;
mod http_utils;
mod jwt;
mod logger;
mod persisted_documents;
mod pipeline;
mod schema_state;
mod shared_state;
mod supergraph;

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
    persisted_documents::PersistedDocumentsLoader,
    pipeline::graphql_request_handler,
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
    mut request: HttpRequest,
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
            &mut request,
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
    let addr = router_config.http.address();
    let mut bg_tasks_manager = BackgroundTasksManager::new();
    let (shared_state, schema_state) =
        configure_app_from_config(router_config, &mut bg_tasks_manager).await?;

    let maybe_error = web::HttpServer::new(move || {
        web::App::new()
            .state(shared_state.clone())
            .state(schema_state.clone())
            .configure(|service_config| {
                configure_ntex_app(service_config, &shared_state.router_config);
            })
            .default_service(web::to(landing_page_handler))
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

    let persisted_docs = if router_config.persisted_documents.enabled {
        Some(PersistedDocumentsLoader::try_new(
            &router_config.persisted_documents,
        )?)
    } else {
        None
    };

    let router_config_arc = Arc::new(router_config);
    let schema_state =
        SchemaState::new_from_config(bg_tasks_manager, router_config_arc.clone()).await?;
    let schema_state_arc = Arc::new(schema_state);
    let shared_state = Arc::new(RouterSharedState::new(
        router_config_arc,
        jwt_runtime,
        persisted_docs,
    )?);

    Ok((shared_state, schema_state_arc))
}

pub fn configure_ntex_app(service_config: &mut web::ServiceConfig, router_config: &HiveRouterConfig) {
    service_config.route(&router_config.http.graphql_endpoint, web::to(graphql_endpoint_handler))
        .route("/health", web::to(health_check_handler))
        .route("/readiness", web::to(readiness_check_handler));
}
