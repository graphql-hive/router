mod http_utils;
mod logger;
mod pipeline;
mod shared_state;
mod supergraph;
mod supergraph_mgr;

use std::sync::Arc;

use crate::{
    http_utils::{health::health_check_handler, landing_page::landing_page_handler},
    logger::configure_logging,
    pipeline::graphql_request_handler,
    shared_state::RouterSharedState,
    supergraph_mgr::SupergraphManager,
};

use hive_router_config::load_config;
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};

async fn graphql_endpoint_handler(
    mut request: HttpRequest,
    body_bytes: Bytes,
    supergraph_manager: web::types::State<Arc<SupergraphManager>>,
    app_state: web::types::State<Arc<RouterSharedState>>,
) -> impl web::Responder {
    let supergraph = supergraph_manager.current();

    graphql_request_handler(&mut request, body_bytes, &supergraph, app_state.get_ref()).await
}

pub async fn router_entrypoint() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::var("ROUTER_CONFIG_FILE_PATH").ok();
    let router_config = load_config(config_path)?;
    configure_logging(&router_config.log);

    let addr = router_config.http.address();

    let supergraph_manager = Arc::new(SupergraphManager::new_from_config(&router_config).await?);
    let shared_state = Arc::new(RouterSharedState::new(router_config));

    web::HttpServer::new(move || {
        web::App::new()
            .state(shared_state.clone())
            .state(supergraph_manager.clone())
            .route("/graphql", web::to(graphql_endpoint_handler))
            .route("/health", web::to(health_check_handler))
            .default_service(web::to(landing_page_handler))
    })
    .bind(addr)?
    .run()
    .await
    .map_err(|err| err.into())
}
