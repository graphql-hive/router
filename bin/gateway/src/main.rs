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
use axum::{
    extract::{Request, State},
    response::IntoResponse,
    routing::{self},
    Router,
};
use gateway_config::load_config;
use mimalloc::MiMalloc;
use tokio::signal;

use query_planner::utils::parsing::parse_schema;
use tokio::net::TcpListener;
use tracing::info;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

async fn graphql_endpoint_handler(
    State(app_state): State<Arc<GatewaySharedState>>,
    mut request: Request,
) -> impl IntoResponse {
    graphql_request_handler(&mut request, app_state).await
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::var("HIVE_CONFIG_FILE_PATH").ok();
    let gateway_config = load_config(config_path)?;
    configure_logging(&gateway_config.log);

    let supergraph_sdl = gateway_config.supergraph.load().await?;
    let parsed_schema = parse_schema(&supergraph_sdl);
    let addr = gateway_config.http.address();
    let gateway_shared_state = GatewaySharedState::new(parsed_schema, gateway_config);

    let app = Router::new()
        .route("/graphql", routing::any(graphql_endpoint_handler))
        .route("/health", routing::get(health_check_handler))
        .fallback(routing::get(landing_page_handler))
        .with_state(gateway_shared_state);

    info!("Starting server on {}", addr);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
