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
use std::{env::var, net::SocketAddr, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::oneshot::{self, Receiver, Sender},
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
    // If body is not valid JSON, store the error message as string
    let body_value: Value =
        sonic_rs::from_slice(&request_body).unwrap_or_else(|err| Value::from(&err.to_string()));

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

#[derive(Clone)]
pub struct SubgraphsServiceState {
    pub request_log: DashMap<String, Vec<RequestLog>>,
    pub health_check_endpoint: String,
}

pub fn start_subgraphs_server(
    port: Option<u16>,
) -> (
    JoinHandle<()>,
    Sender<()>,
    Arc<SubgraphsServiceState>,
    Receiver<(String, u16)>,
) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (addr_tx, addr_rx) = oneshot::channel::<(String, u16)>();
    let host = var("HOST").unwrap_or("0.0.0.0".to_owned());
    let port_input = port.unwrap_or_else(|| {
        var("PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(4200)
    });

    let shared_state = Arc::new(SubgraphsServiceState {
        request_log: DashMap::new(),
        health_check_endpoint: "health".to_string(),
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

    let server_handle = tokio::spawn(async move {
        let listener = TcpListener::bind(&format!("{}:{}", host, port_input))
            .await
            .unwrap();

        let assigned_addr: SocketAddr = listener.local_addr().unwrap();
        let assigned_host = assigned_addr.ip().to_string();
        let assigned_port = assigned_addr.port();

        println!(
            "Starting server on http://{}:{}",
            assigned_host, assigned_port
        );

        // Send the assigned host and port back to the caller
        let _ = addr_tx.send((assigned_host, assigned_port));

        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                shutdown_rx.await.ok();
                println!("Graceful shutdown signal received.");
            })
            .await
            .expect("failed to start subgraphs server");
    });

    (server_handle, shutdown_tx, shared_state, addr_rx)
}
