pub mod accounts;
pub mod inventory;
pub mod products;
pub mod reviews;

use async_graphql_axum::GraphQL;
use axum::{
    body::{to_bytes, Bytes},
    extract::{Request, State},
    http::{self, request::Parts, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post_service},
    Router,
};
use dashmap::DashMap;
use sonic_rs::Value;
use std::{env::var, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::oneshot::{self, Sender},
    task::JoinHandle,
};

extern crate lazy_static;

async fn delay_middleware(req: Request, next: Next) -> Response {
    let delay_ms: Option<u64> = std::env::var("SUBGRAPH_DELAY_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|d| *d != 0);

    if let Some(delay_ms) = delay_ms {
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
    }

    next.run(req).await
}

async fn add_subgraph_header(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let subgraph_name = path.trim_start_matches('/').to_string();

    let mut response = next.run(req).await;

    if !subgraph_name.is_empty() && subgraph_name != "health" {
        if let Ok(header_value) = subgraph_name.parse() {
            response.headers_mut().insert("x-subgraph", header_value);
        }
    }

    response
}

async fn track_requests(
    State(state): State<Arc<SubgraphsServiceState>>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let path = request.uri().path().to_string();
    let (parts, body) = request.into_parts();
    let body_bytes = to_bytes(body, usize::MAX).await.unwrap();
    let record = extract_record(&parts, body_bytes.clone());

    state.request_log.entry(path).or_default().push(record);
    let new_body = axum::body::Body::from(body_bytes);
    let request = Request::from_parts(parts, new_body);

    next.run(request).await
}

fn extract_record(request_parts: &Parts, request_body: Bytes) -> RequestLog {
    let header_map = request_parts.headers.clone();
    let body_value: Value = sonic_rs::from_slice(&request_body).unwrap_or(Value::new());

    RequestLog {
        headers: header_map,
        request_body: body_value,
    }
}

async fn health_check_handler() -> impl IntoResponse {
    StatusCode::OK
}

#[derive(Debug, Clone, Default)]
pub struct RequestLog {
    pub headers: http::HeaderMap,
    pub request_body: Value,
}

pub struct SubgraphsServiceState {
    pub request_log: DashMap<String, Vec<RequestLog>>,
    pub health_check_url: String,
}

pub fn start_subgraphs_server(
    port: Option<u16>,
) -> (JoinHandle<()>, Sender<()>, Arc<SubgraphsServiceState>) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let host = var("HOST").unwrap_or("0.0.0.0".to_owned());
    let port = port
        .map(|v| v.to_string())
        .unwrap_or(var("PORT").unwrap_or("4200".to_owned()));

    let shared_state = Arc::new(SubgraphsServiceState {
        request_log: DashMap::new(),
        health_check_url: format!("http://{}:{}/health", host, port),
    });

    let app = Router::new()
        .route(
            "/accounts",
            post_service(GraphQL::new(accounts::get_subgraph())),
        )
        .route(
            "/inventory",
            post_service(GraphQL::new(inventory::get_subgraph())),
        )
        .route(
            "/products",
            post_service(GraphQL::new(products::get_subgraph())),
        )
        .route(
            "/reviews",
            post_service(GraphQL::new(reviews::get_subgraph())),
        )
        .layer(middleware::from_fn_with_state(
            shared_state.clone(),
            track_requests,
        ))
        .route("/health", get(health_check_handler))
        .route_layer(middleware::from_fn(add_subgraph_header))
        .route_layer(middleware::from_fn(delay_middleware));

    println!("Starting server on http://{}:{}", host, port);

    let server_handle = tokio::spawn(async move {
        axum::serve(
            TcpListener::bind(&format!("{}:{}", host, port))
                .await
                .unwrap(),
            app,
        )
        .with_graceful_shutdown(async {
            shutdown_rx.await.ok();
            println!("Graceful shutdown signal received.");
        })
        .await
        .expect("failed to start subgraphs server");
    });

    (server_handle, shutdown_tx, shared_state)
}
