mod http_utils;
mod logger;
mod pipeline;
mod shared_state;

use std::sync::Arc;

use crate::{
    http_utils::{health::health_check_handler, landing_page::landing_page_handler},
    logger::configure_logging,
    pipeline::graphql_request_handler,
    shared_state::GatewaySharedState,
};

use gateway_config::load_config;
use mimalloc::MiMalloc;
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};

use query_planner::utils::parsing::parse_schema;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

async fn graphql_endpoint_handler(
    mut request: HttpRequest,
    body_bytes: Bytes,
    app_state: web::types::State<Arc<GatewaySharedState>>,
) -> impl web::Responder {
    graphql_request_handler(&mut request, body_bytes, app_state.get_ref()).await
}

#[ntex::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::var("HIVE_CONFIG_FILE_PATH").ok();
    let gateway_config = load_config(config_path)?;
    configure_logging(&gateway_config.log);

    let supergraph_sdl = gateway_config.supergraph.load().await?;
    let parsed_schema = parse_schema(&supergraph_sdl);
    let addr = gateway_config.http.address();
    let gateway_shared_state = GatewaySharedState::new(parsed_schema, gateway_config);

    web::HttpServer::new(move || {
        web::App::new()
            .state(gateway_shared_state.clone())
            .route("/graphql", web::to(graphql_endpoint_handler))
            .route("/health", web::to(health_check_handler))
            .default_service(web::to(landing_page_handler))
    })
    .bind(addr)?
    .run()
    .await?;

    Ok(())
}
