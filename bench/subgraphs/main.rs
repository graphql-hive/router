use async_graphql_axum::GraphQL;
use axum::{
    extract::Request,
    middleware::{self, Next},
    response::Response,
    routing::post_service,
    Router,
};
use std::env::var;
use tokio::net::TcpListener;

mod accounts;
mod inventory;
mod products;
mod reviews;

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

#[tokio::main]
async fn main() {
    let host = var("HOST").unwrap_or("0.0.0.0".to_owned());
    let port = var("PORT").unwrap_or("4200".to_owned());

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
        .route_layer(middleware::from_fn(delay_middleware));

    println!("Starting server on http://localhost:4200");

    axum::serve(
        TcpListener::bind(&format!("{}:{}", host, port))
            .await
            .unwrap(),
        app,
    )
    .await
    .unwrap();
}
