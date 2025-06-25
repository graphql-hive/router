mod http_utils;
mod logger;
mod pipeline;
mod shared_state;

use crate::{
    http_utils::{
        landing_page::landing_page_handler,
        request_id::{RequestIdGenerator, REQUEST_ID_HEADER_NAME},
    },
    logger::{configure_logging, LoggingFormat},
    shared_state::GatewaySharedState,
};
use axum::{
    body::Body,
    http::Method,
    routing::{any_service, get},
    Router,
};
use http::Request;
use tokio::signal;

use axum::Extension;
use tower::ServiceBuilder;
use tracing::debug_span;

use crate::pipeline::{
    coerce_variables_service::CoerceVariablesService, execution_service::ExecutionService,
    graphiql_service::GraphiQLResponderService,
    graphql_request_params::GraphQLRequestParamsExtractor,
    http_request_params::HttpRequestParamsExtractor,
    normalize_service::GraphQLOperationNormalizationService, parser_service::GraphQLParserService,
    query_plan_service::QueryPlanService, validation_service::GraphQLValidationService,
};
use query_planner::utils::parsing::parse_schema;
use std::{env, net::SocketAddr};
use tokio::net::TcpListener;
use tower_http::{
    cors::CorsLayer,
    request_id::{PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let perfetto_file = env::var("PERFETTO_OUT").ok().is_some_and(|v| v == "1");
    let log_format = env::var("LOG_FORMAT")
        .map(|v| match v.as_str().to_lowercase() {
            str if str == "json" => LoggingFormat::Json,
            str if str == "tree" => LoggingFormat::PrettyTree,
            str if str == "compact" => LoggingFormat::PrettyCompact,
            _ => LoggingFormat::PrettyCompact,
        })
        .unwrap_or(LoggingFormat::PrettyCompact);
    let _logger_drop = configure_logging(log_format, perfetto_file);

    let expose_query_plan = env::var("EXPOSE_QUERY_PLAN")
        .map(|v| matches!(v.as_str().to_lowercase(), str if str == "true" || str == "1"))
        .unwrap_or(false);

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: gateway <SUPERGRAPH_PATH>");
        return Err("Missing supergraph path argument".into());
    }

    let supergraph_path = &args[1];

    let supergraph_sdl =
        std::fs::read_to_string(supergraph_path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let gateway_shared_state = GatewaySharedState::new(parsed_schema);

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
        .layer(HttpRequestParamsExtractor::new_layer())
        .layer(GraphiQLResponderService::new_layer())
        .layer(GraphQLRequestParamsExtractor::new_layer())
        .layer(GraphQLParserService::new_layer())
        .layer(GraphQLOperationNormalizationService::new_layer())
        .layer(CoerceVariablesService::new_layer())
        .layer(GraphQLValidationService::new_layer())
        .layer(QueryPlanService::new_layer())
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER_NAME.clone()))
        .service(ExecutionService::new(expose_query_plan));

    let app = Router::new()
        .route("/graphql", any_service(pipeline))
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

    let addr = SocketAddr::from(([0, 0, 0, 0], 4000));
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
