mod http_utils;
mod logger;
mod pipeline;
mod shared_state;

use crate::{
    http_utils::{
        health::health_check_handler,
        landing_page::landing_page_handler,
        request_id::{RequestIdGenerator, REQUEST_ID_HEADER_NAME},
    },
    logger::configure_logging,
    shared_state::GatewaySharedState,
};
use axum::{
    body::Body,
    http::Method,
    routing::{any_service, get},
    Router,
};
use gateway_config::load_config;
use http::Request;
use mimalloc::MiMalloc;
use tokio::signal;

use axum::Extension;
use tower::ServiceBuilder;

use crate::pipeline::{
    coerce_variables_service::CoerceVariablesService, execution_service::ExecutionService,
    graphiql_service::GraphiQLResponderService,
    graphql_request_params::GraphQLRequestParamsExtractor,
    normalize_service::GraphQLOperationNormalizationService, parser_service::GraphQLParserService,
    progressive_override_service::ProgressiveOverrideExtractor,
    query_plan_service::QueryPlanService, validation_service::GraphQLValidationService,
};
use query_planner::utils::parsing::parse_schema;
use tokio::net::TcpListener;
use tower_http::{
    cors::CorsLayer,
    request_id::{PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::{debug_span, info};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::var("HIVE_CONFIG_FILE_PATH").ok();
    let gateway_config = load_config(config_path)?;
    configure_logging(&gateway_config.log);

    let allow_expose_query_plan = gateway_config.query_planner.allow_expose;
    let supergraph_sdl = gateway_config.supergraph.load().await?;
    let parsed_schema = parse_schema(&supergraph_sdl);
    let gateway_shared_state = GatewaySharedState::new(parsed_schema, &gateway_config);

    let pipeline = ServiceBuilder::new()
        .layer(Extension(gateway_shared_state.clone()))
        .layer(SetRequestIdLayer::new(
            REQUEST_ID_HEADER_NAME.clone(),
            RequestIdGenerator,
        ))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
                let request_id = request
                    .extensions()
                    .get::<RequestId>()
                    .map(|v| v.header_value().to_str().unwrap())
                    .unwrap_or_else(|| "");

                debug_span!(
                    "http_request",
                    request_id = %request_id,
                    method = %request.method(),
                    uri = %request.uri(),
                )
            }),
        )
        .layer(GraphiQLResponderService::new_layer())
        .layer(GraphQLRequestParamsExtractor::new_layer())
        .layer(GraphQLParserService::new_layer())
        .layer(GraphQLValidationService::new_layer())
        .layer(ProgressiveOverrideExtractor::new_layer())
        .layer(GraphQLOperationNormalizationService::new_layer())
        .layer(CoerceVariablesService::new_layer())
        .layer(QueryPlanService::new_layer())
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER_NAME.clone()))
        .service(ExecutionService::new(allow_expose_query_plan));

    let app = Router::new()
        .route("/graphql", any_service(pipeline))
        .route("/health", get(health_check_handler))
        .layer(
            CorsLayer::new()
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers(vec![
                    axum::http::header::ACCEPT,
                    axum::http::header::CONTENT_TYPE,
                ])
                .allow_origin(tower_http::cors::Any),
        )
        .fallback(get(landing_page_handler))
        .with_state(gateway_shared_state);

    let addr = gateway_config.http.address();
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
