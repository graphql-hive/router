mod background_tasks;
mod context;
mod http_utils;
mod jwt;
mod logger;
mod pipeline;
mod shared_state;

use std::sync::Arc;

use crate::{
    background_tasks::BackgroundTasksManager,
    context::RequestContext,
    http_utils::{health::health_check_handler, landing_page::landing_page_handler},
    jwt::JwtAuthRuntime,
    logger::configure_logging,
    pipeline::graphql_request_handler,
    shared_state::RouterSharedState,
};

use hive_router_config::load_config;
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};

use hive_router_query_planner::utils::parsing::parse_schema;
use tracing::info;

async fn graphql_endpoint_handler(
    mut request: HttpRequest,
    body_bytes: Bytes,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> impl web::Responder {
    request.extensions_mut().insert(RequestContext::new());
    graphql_request_handler(&mut request, body_bytes, app_state.get_ref()).await
}

pub async fn router_entrypoint() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::var("ROUTER_CONFIG_FILE_PATH").ok();
    let router_config = load_config(config_path)?;
    configure_logging(&router_config.log);

    let supergraph_sdl = router_config.supergraph.load().await?;
    let parsed_schema = parse_schema(&supergraph_sdl);
    let addr = router_config.http.address();
    let mut bg_tasks_manager = BackgroundTasksManager::new();

    let jwt_runtime = if let Some(jwt_config) = &router_config.jwt {
        Some(JwtAuthRuntime::init(&mut bg_tasks_manager, jwt_config).await?)
    } else {
        None
    };

    let shared_state = RouterSharedState::new(parsed_schema, router_config, jwt_runtime);

    let maybe_error = web::HttpServer::new(move || {
        web::App::new()
            .state(shared_state.clone())
            .route("/graphql", web::to(graphql_endpoint_handler))
            .route("/health", web::to(health_check_handler))
            .default_service(web::to(landing_page_handler))
    })
    .bind(addr)?
    .run()
    .await
    .map_err(|err| err.into());

    info!("server stopped, clearning background tasks");
    bg_tasks_manager.shutdown().await;

    maybe_error
}
