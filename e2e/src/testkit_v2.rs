use std::{
    any::Any, marker::PhantomData, net::SocketAddr, str::FromStr, sync::Arc, time::Duration,
};

use axum;
use dashmap::DashMap;
use hive_router::{
    background_tasks::BackgroundTasksManager, configure_app_from_config, configure_ntex_app,
    init_rustls_crypto_provider, telemetry::Telemetry,
};
use hive_router_config::{parse_yaml_config, HiveRouterConfig};
use hive_router_plan_executor::executors::websocket_client;
use ntex::{
    client::ClientResponse,
    io::Sealed,
    web::{self, test},
    ws::WsConnection,
};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use sonic_rs::json;
use subgraphs::{subgraphs_app, SubscriptionProtocol};
use tokio::{net::TcpListener, sync::oneshot};
use tracing::{info, warn};

pub struct Built;
pub struct Started;

// subgraphs

pub struct TestSubgraphsBuilder {
    subscriptions_protocol: SubscriptionProtocol,
}

impl TestSubgraphsBuilder {
    pub fn new() -> Self {
        Self {
            subscriptions_protocol: SubscriptionProtocol::Auto,
        }
    }

    pub fn build(self) -> TestSubgraphs<Built> {
        TestSubgraphs {
            subscriptions_protocol: self.subscriptions_protocol,
            handle: None,
            _state: PhantomData,
        }
    }
}

impl Default for TestSubgraphsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

struct TestSubgraphsHandle {
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    addr: SocketAddr,
    state: Arc<TestSubgraphsMiddlewareState>,
}

pub struct TestSubgraphs<State> {
    subscriptions_protocol: SubscriptionProtocol,
    handle: Option<TestSubgraphsHandle>,
    _state: PhantomData<State>,
}

#[derive(Clone)]
pub struct RequestLog {
    pub headers: http::HeaderMap,
    #[allow(unused)]
    pub body: sonic_rs::Value,
}

struct TestSubgraphsMiddlewareState {
    /// A map of request path to list of requests received on that path.
    request_log: DashMap<String, Vec<RequestLog>>,
}

async fn record_requests(
    axum::extract::State(state): axum::extract::State<Arc<TestSubgraphsMiddlewareState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> impl axum::response::IntoResponse {
    let path = request.uri().path().to_string();
    let (parts, body) = request.into_parts();
    let body_bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();

    let header_map = parts.headers.clone();
    let body_value: sonic_rs::Value = sonic_rs::from_slice(&body_bytes)
        .unwrap_or_else(|err| sonic_rs::Value::from(&err.to_string()));
    let record = RequestLog {
        headers: header_map,
        body: body_value,
    };
    state.request_log.entry(path).or_default().push(record);

    let rebuilt_body = axum::body::Body::from(body_bytes);
    let request = axum::extract::Request::from_parts(parts, rebuilt_body);
    next.run(request).await
}

impl TestSubgraphs<Built> {
    pub async fn start(self) -> TestSubgraphs<Started> {
        let listener = TcpListener::bind("127.0.0.1:4200") // TODO: use 0 and allocate random port
            .await
            .expect("failed to bind tcp listener");
        let addr = listener.local_addr().expect("failed to get local address");

        let mut app = subgraphs_app(self.subscriptions_protocol.clone());

        let middleware_state = Arc::new(TestSubgraphsMiddlewareState {
            request_log: DashMap::new(),
        });
        app = app.layer(axum::middleware::from_fn_with_state(
            middleware_state.clone(),
            record_requests,
        ));

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
                .expect("failed to start subgraphs server");
        });

        TestSubgraphs {
            subscriptions_protocol: self.subscriptions_protocol,
            handle: Some(TestSubgraphsHandle {
                shutdown_tx: Some(shutdown_tx),
                addr,
                state: middleware_state,
            }),
            _state: PhantomData,
        }
    }
}

impl TestSubgraphs<Started> {
    #[allow(unused)]
    pub fn addr(&self) -> SocketAddr {
        self.handle.as_ref().expect("subgraphs not started").addr
    }

    pub fn get_requests_log(&self, path: &str) -> Option<Vec<RequestLog>> {
        self.handle
            .as_ref()
            .expect("subgraphs not started")
            .state
            .request_log
            .get(path)
            .map(|entry| entry.value().to_vec())
    }
}

impl Drop for TestSubgraphsHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.take().map(|tx| tx.send(()));
    }
}

// router

pub struct TestRouterBuilder<'subgraphs> {
    config: Option<HiveRouterConfig>,
    subgraphs: Option<&'subgraphs TestSubgraphs<Started>>,
}

impl<'subgraphs> TestRouterBuilder<'subgraphs> {
    pub fn new() -> Self {
        Self {
            config: None,
            subgraphs: None,
        }
    }

    pub fn inline_config(mut self, config_yaml: &str) -> Self {
        let router_config = parse_yaml_config(config_yaml.to_string()).unwrap();
        self.config = Some(router_config);
        self
    }

    pub fn with_subgraphs(mut self, subgraphs: &'subgraphs TestSubgraphs<Started>) -> Self {
        self.subgraphs = Some(subgraphs);
        self
    }

    pub fn build(self) -> TestRouter<'subgraphs, Built> {
        let config = self.config.expect("config is required");
        TestRouter {
            graphql_path: config.graphql_path().to_string(),
            websocket_path: config.websocket_path().map(|p| p.to_string()),
            config: Some(config),
            subgraphs: self.subgraphs,
            handle: None,
            _hold_until_drop: vec![],
            _state: PhantomData,
        }
    }
}

impl Default for TestRouterBuilder<'_> {
    fn default() -> Self {
        Self::new()
    }
}

struct TestRouterHandle {
    serv: test::TestServer,
    bg_tasks_manager: BackgroundTasksManager,
}

impl Drop for TestRouterHandle {
    fn drop(&mut self) {
        self.bg_tasks_manager.shutdown();
    }
}

pub struct TestRouter<'subgraphs, State> {
    graphql_path: String,
    websocket_path: Option<String>,
    config: Option<HiveRouterConfig>,
    subgraphs: Option<&'subgraphs TestSubgraphs<Started>>,
    handle: Option<TestRouterHandle>,
    _hold_until_drop: Vec<Box<dyn Any>>,
    _state: PhantomData<State>,
}

impl<'subgraphs> TestRouter<'subgraphs, Built> {
    pub async fn start(mut self) -> TestRouter<'subgraphs, Started> {
        init_rustls_crypto_provider();
        let config = self.config.take().unwrap();
        let (telemetry, subscriber) =
            Telemetry::init_subscriber(&config).expect("failed to initialize telemetry subscriber");
        let subscription_guard = tracing::subscriber::set_default(subscriber);

        let mut bg_tasks_manager = BackgroundTasksManager::new();
        let (shared_state, schema_state) =
            configure_app_from_config(config, telemetry.context.clone(), &mut bg_tasks_manager)
                .await
                .expect("failed to configure hive router from config");

        let serv_graphql_path = self.graphql_path.clone();
        let serv_websocket_path = self.websocket_path.clone();
        let serv = test::server(move || {
            let shared_state = shared_state.clone();
            let schema_state = schema_state.clone();
            let serv_graphql_path = serv_graphql_path.clone();
            let serv_websocket_path = serv_websocket_path.clone();
            async move {
                web::App::new()
                    .state(shared_state)
                    .state(schema_state)
                    .configure(|m| {
                        configure_ntex_app(
                            m,
                            serv_graphql_path.as_ref(),
                            serv_websocket_path.as_deref(),
                        )
                    })
            }
        })
        .await;

        info!("Waiting for health check to pass...");

        loop {
            match serv.get("/health").send().await {
                Ok(response) => {
                    if response.status() == 200 {
                        break;
                    }
                }
                Err(err) => {
                    warn!("Server not healthy yet, retrying in 50ms: {:?}", err);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }

        info!("Waiting for readiness check to pass...");

        loop {
            match serv.get("/readiness").send().await {
                Ok(response) => {
                    if response.status() == 200 {
                        break;
                    }
                }
                Err(err) => {
                    warn!("Server not ready yet, retrying in 50ms: {:?}", err);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }

        TestRouter {
            graphql_path: self.graphql_path,
            websocket_path: self.websocket_path,
            handle: Some(TestRouterHandle {
                serv,
                bg_tasks_manager,
            }),
            config: self.config,
            subgraphs: self.subgraphs,
            _hold_until_drop: vec![Box::new(subscription_guard)],
            _state: PhantomData,
        }
    }
}

impl<'subgraphs> TestRouter<'subgraphs, Started> {
    #[allow(unused)]
    pub async fn send_graphql_request(
        &self,
        query: &str,
        variables: Option<sonic_rs::Value>,
    ) -> ClientResponse {
        let req = self
            .handle
            .as_ref()
            .unwrap()
            .serv
            .post(self.graphql_path.as_str())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/graphql-response+json");

        req.send_json(&json!({
          "query": query,
          "variables": variables,
        }))
        .await
        .expect("Failed to send graphql request")
    }

    pub async fn ws(&self) -> WsConnection<Sealed> {
        let url = self.handle.as_ref().unwrap().serv.url(
            self.websocket_path
                .as_ref()
                .expect("Websocket path not set"),
        );
        let ws_url = url.as_str().replace("http://", "ws://");
        let ws_uri = http::Uri::from_str(&ws_url).expect("Failed to parse ws url");
        websocket_client::connect(&ws_uri)
            .await
            .expect("Failed to connect to websocket")
    }
}
