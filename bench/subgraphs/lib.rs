pub mod accounts;
pub mod books;
pub mod graphql_with_subscriptions;
pub mod inventory;
pub mod monolith;
pub mod products;
pub mod reviews;

use std::{
    collections::HashMap,
    sync::{atomic::AtomicUsize, Arc, LazyLock},
    time::Duration,
};

use async_graphql_axum::{GraphQL, GraphQLSubscription};
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post_service},
    Router,
};
use tokio::{
    net::TcpListener,
    sync::oneshot::{self, Sender},
    task::JoinHandle,
};

/// Artificial latency per subgraph, read once at startup.
///
/// `SUBGRAPH_DELAY_MS` delays every subgraph, `SUBGRAPH_DELAY_MS_<NAME>` overrides it for
/// one (`SUBGRAPH_DELAY_MS_REVIEWS=100`). A uniform delay makes every fetch equally slow,
/// which is exactly the case where wave execution and dependency-aware execution behave
/// the same - to see a wave barrier you need the subgraphs to differ.
///
/// Read once rather than per request: an env lookup per request lands in the latency the
/// benchmark is measuring.
static SUBGRAPH_DELAYS: LazyLock<SubgraphDelays> = LazyLock::new(SubgraphDelays::from_env);

struct SubgraphDelays {
    every_subgraph: Option<u64>,
    by_subgraph: HashMap<String, u64>,
}

impl SubgraphDelays {
    fn from_env() -> Self {
        fn parse(value: String) -> Option<u64> {
            value.parse::<u64>().ok().filter(|ms| *ms != 0)
        }

        let by_subgraph = std::env::vars()
            .filter_map(|(key, value)| {
                let name = key.strip_prefix("SUBGRAPH_DELAY_MS_")?;
                Some((name.to_ascii_lowercase(), parse(value)?))
            })
            .collect();

        Self {
            every_subgraph: std::env::var("SUBGRAPH_DELAY_MS").ok().and_then(parse),
            by_subgraph,
        }
    }

    /// `/reviews` and `/reviews/ws` are both the `reviews` subgraph.
    fn for_path(&self, path: &str) -> Option<Duration> {
        let subgraph = path.trim_start_matches('/').split('/').next()?;

        self.by_subgraph
            .get(subgraph)
            .copied()
            .or(self.every_subgraph)
            .map(Duration::from_millis)
    }
}

async fn delay_middleware(req: Request, next: Next) -> Response {
    if let Some(delay) = SUBGRAPH_DELAYS.for_path(req.uri().path()) {
        tokio::time::sleep(delay).await;
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

async fn health_check_handler() -> impl IntoResponse {
    StatusCode::OK
}

pub fn start_subgraphs_server(port: Option<u16>) -> (JoinHandle<()>, Sender<()>) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let host = std::env::var("HOST").unwrap_or("0.0.0.0".to_owned());
    let port = port
        .map(|v| v.to_string())
        .unwrap_or(std::env::var("PORT").unwrap_or("4200".to_owned()));

    let (mut app, _) = subgraphs_app(HTTPStreamingSubscriptionProtocol::default());
    app = app.route("/health", get(health_check_handler));

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

    (server_handle, shutdown_tx)
}

/// The protocol to use for GraphQL subscriptions over HTTP streaming.
/// It is purely the streaming HTTP protocol, other subscription protocols
/// are handled automatically through HTTP negotiation (like websocket upgrades
/// or http callbacks).
#[derive(Clone, Default)]
pub enum HTTPStreamingSubscriptionProtocol {
    #[default]
    PreferMultipartFallbackSse,
    MultipartOnly,
    SseOnly,
}

pub fn subgraphs_app(
    subscriptions_protocol: HTTPStreamingSubscriptionProtocol,
) -> (Router<()>, Arc<AtomicUsize>) {
    let accounts_schema = accounts::get_subgraph();
    let inventory_schema = inventory::get_subgraph();
    let products_schema = products::get_subgraph();
    let (reviews_schema, active_subscriptions) = reviews::get_subgraph();
    let router = Router::new()
        .route_service(
            "/accounts/ws",
            GraphQLSubscription::new(accounts_schema.clone()),
        )
        .route("/accounts", post_service(GraphQL::new(accounts_schema)))
        .route("/books", post_service(GraphQL::new(books::get_subgraph())))
        .route_service(
            "/inventory/ws",
            GraphQLSubscription::new(inventory_schema.clone()),
        )
        .route("/inventory", post_service(GraphQL::new(inventory_schema)))
        .route_service(
            "/products/ws",
            GraphQLSubscription::new(products_schema.clone()),
        )
        .route("/products", post_service(GraphQL::new(products_schema)))
        .route_service(
            "/reviews/ws",
            GraphQLSubscription::new(reviews_schema.clone()),
        )
        .route(
            "/reviews",
            post_service(graphql_with_subscriptions::GraphQL::new(
                reviews_schema,
                subscriptions_protocol,
            )),
        )
        .route(
            "/monolith",
            post_service(GraphQL::new(monolith::get_schema())),
        )
        .route_layer(middleware::from_fn(add_subgraph_header))
        .route_layer(middleware::from_fn(delay_middleware));
    (router, active_subscriptions)
}
