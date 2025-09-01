mod http_utils;
mod logger;
mod pipeline;
mod shared_state;

use std::sync::Arc;

use crate::{
    http_utils::{health::health_check_handler, landing_page::landing_page_handler},
    logger::configure_logging,
    pipeline::graphql_request_handler,
    shared_state::RouterSharedState,
};

use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};
use router_config::load_config;

use hive_router_query_planner::utils::parsing::parse_schema;

async fn graphql_endpoint_handler(
    mut request: HttpRequest,
    body_bytes: Bytes,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> impl web::Responder {
    graphql_request_handler(&mut request, body_bytes, app_state.get_ref()).await
}

pub async fn router_entrypoint() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::var("HIVE_CONFIG_FILE_PATH").ok();
    let router_config = load_config(config_path)?;
    configure_logging(&router_config.log);

    let supergraph_sdl = router_config.supergraph.load().await?;
    let parsed_schema = parse_schema(&supergraph_sdl);
    let addr = router_config.http.address();
    let shared_state = RouterSharedState::new(parsed_schema, router_config);

    web::HttpServer::new(move || {
        web::App::new()
            .state(shared_state.clone())
            .route("/graphql", web::to(graphql_endpoint_handler))
            .route("/health", web::to(health_check_handler))
            .default_service(web::to(landing_page_handler))
    })
    .bind(addr)?
    .run()
    .await
    .map_err(|err| err.into())
}
