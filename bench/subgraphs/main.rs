use async_graphql_axum::GraphQL;
use axum::{
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::post_service,
    Router, Server,
};
use std::env::var;

mod accounts;
mod inventory;
mod products;
mod reviews;

extern crate lazy_static;

async fn delay_middleware<B>(req: Request<B>, next: Next<B>) -> Result<Response, StatusCode> {
    let delay_ms: Option<u64> = std::env::var("SUBGRAPH_DELAY_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|d| *d != 0);

    if let Some(delay_ms) = delay_ms {
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
    }

    Ok(next.run(req).await)
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
    Server::bind(&format!("{}:{}", host, port).parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
