use axum;
use bytes::Bytes;
use dashmap::DashMap;
use lazy_static::lazy_static;
use ntex::{
    client::ClientResponse,
    io::Sealed,
    web::{self, test},
    ws::WsConnection,
};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use sonic_rs::json;
use std::{
    any::Any, marker::PhantomData, net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc,
    time::Duration,
};
use tempfile::NamedTempFile;
use tokio::{
    net::TcpListener,
    sync::{oneshot, Semaphore},
};
use tracing::{info, warn};

lazy_static! {
    /// Limits concurrent test routers to avoid hitting the OS open-file-descriptor limit.
    /// Hive Router and the subgraphs opens about 24 file descriptors per instance (tokio
    /// for the event queue, dyn libs, port bindings, domain sockets), so we divide the system's
    /// `RLIMIT_NOFILE` by that number to get a safe concurrency limit. Increasing the OS ulimit
    /// will increase the concurrency of tests.
    static ref CONCURRENCY_SEMAPHORE: Arc<Semaphore> = {
        let limit = {
            let mut rlimit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            let nofile = if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlimit) } == 0 {
                rlimit.rlim_cur as usize
            } else {
                256 // fallback, about the default ulimit on many sysstms
            };
            (nofile / 24).max(1)
        };
        info!("Concurrency semaphore initialized with {} permits", limit);
        Arc::new(Semaphore::new(limit))
    };
}

use hive_router::{
    background_tasks::BackgroundTasksManager, configure_app_from_config, configure_ntex_app,
    init_rustls_crypto_provider, telemetry::Telemetry,
};
use hive_router_config::{load_config, parse_yaml_config, HiveRouterConfig};
use hive_router_plan_executor::executors::websocket_client;
use subgraphs::{subgraphs_app, SubscriptionProtocol};

// utilities

/// Creates a Some(http::HeaderMap) from a list of key-value pairs, for use in test requests.
#[macro_export]
macro_rules! some_header_map {
    ($($key:expr => $val:expr),* $(,)?) => {{
        let mut map = ::http::HeaderMap::new();
        $(map.insert($key, $val.parse().unwrap());)*
        Some(map)
    }};
}

// #[macro_export] always hoists to the crate root so we re-export it here module level
pub use some_header_map;

// state markers

pub struct Built;
pub struct Started;

// subgraphs

#[derive(Clone)]
pub struct RequestLike {
    #[allow(unused)]
    pub path: String,
    pub headers: http::HeaderMap,
    #[allow(unused)]
    pub body: Option<Bytes>,
}

pub struct ResponseLike {
    pub status: axum::http::StatusCode,
    pub headers: http::HeaderMap,
    pub body: Option<Bytes>,
}

impl ResponseLike {
    pub fn new(
        status: axum::http::StatusCode,
        body: Option<String>,
        headers: Option<http::HeaderMap>,
    ) -> Self {
        Self {
            status,
            headers: headers.unwrap_or_else(http::HeaderMap::new),
            body: body.map(Bytes::from),
        }
    }
}

type OnRequest = dyn Fn(RequestLike) -> Option<ResponseLike> + Send + Sync;

pub struct TestSubgraphsBuilder {
    subscriptions_protocol: SubscriptionProtocol,
    on_request: Option<Arc<OnRequest>>,
}

impl TestSubgraphsBuilder {
    pub fn new() -> Self {
        Self {
            on_request: None,
            subscriptions_protocol: SubscriptionProtocol::default(),
        }
    }

    pub fn with_subscriptions_protocol(mut self, protocol: SubscriptionProtocol) -> Self {
        self.subscriptions_protocol = protocol;
        self
    }

    pub fn with_on_request(
        mut self,
        on_request: impl Fn(RequestLike) -> Option<ResponseLike> + Send + Sync + 'static,
    ) -> Self {
        self.on_request = Some(Arc::new(on_request));
        self
    }

    pub fn build(self) -> TestSubgraphs<Built> {
        TestSubgraphs {
            subscriptions_protocol: self.subscriptions_protocol,
            on_request: self.on_request,
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
    on_request: Option<Arc<OnRequest>>,
    handle: Option<TestSubgraphsHandle>,
    _state: PhantomData<State>,
}

struct TestSubgraphsMiddlewareState {
    /// A map of subgraph name to list of requests received on that subgraph.
    request_log: DashMap<String, Vec<RequestLike>>,
}

async fn record_requests(
    axum::extract::State(state): axum::extract::State<Arc<TestSubgraphsMiddlewareState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> impl axum::response::IntoResponse {
    let path = request.uri().path().to_string();
    let subgraph = path
        .trim_start_matches("/") // remove leading slash to have the path represent the subgraph
        .to_string();
    let (parts, body) = request.into_parts();
    let body_bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();

    let header_map = parts.headers.clone();
    let record = RequestLike {
        path,
        headers: header_map,
        body: body_bytes
            .is_empty()
            .then(|| None)
            .unwrap_or(Some(body_bytes.clone())),
    };
    state.request_log.entry(subgraph).or_default().push(record);

    let rebuilt_body = axum::body::Body::from(body_bytes);
    let request = axum::extract::Request::from_parts(parts, rebuilt_body);
    next.run(request).await
}

async fn handle_on_request(
    axum::extract::State(on_request): axum::extract::State<Arc<OnRequest>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> impl axum::response::IntoResponse {
    let path = request.uri().path().to_string();
    let (parts, body) = request.into_parts();

    let req = RequestLike {
        path: path.clone(),
        headers: parts.headers.clone(),
        body: None, // TODO: do we really care about the body?
    };

    if let Some(new_resp) = on_request(req) {
        // response intercepted, return it and stop
        let mut response = axum::response::Response::builder()
            .status(new_resp.status)
            .body(if let Some(body) = new_resp.body {
                axum::body::Body::from(body)
            } else {
                axum::body::Body::empty()
            })
            .unwrap();
        *response.headers_mut() = new_resp.headers;
        return response;
    }

    let request = axum::extract::Request::from_parts(parts, body);
    next.run(request).await
}

impl TestSubgraphs<Built> {
    pub async fn start(self) -> TestSubgraphs<Started> {
        let listener = TcpListener::bind("127.0.0.1:0")
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
        if let Some(on_request) = self.on_request.clone() {
            app = app.layer(axum::middleware::from_fn_with_state(
                on_request,
                handle_on_request,
            ));
        }

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
            on_request: self.on_request,
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

    /// Returns the list of requests received on the given subgraph. Supply the subgarph name.
    pub fn get_requests_log(&self, subgraph: &str) -> Option<Vec<RequestLike>> {
        self.handle
            .as_ref()
            .expect("subgraphs not started")
            .state
            .request_log
            .get(subgraph)
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

    pub fn file_config(mut self, config_path: &str) -> Self {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(config_path);
        self.config = Some(
            load_config(Some(supergraph_path.to_str().unwrap().to_string()))
                .expect("failed to load router config from file"),
        );
        self
    }

    pub fn with_subgraphs(mut self, subgraphs: &'subgraphs TestSubgraphs<Started>) -> Self {
        self.subgraphs = Some(subgraphs);
        self
    }

    pub fn build(self) -> TestRouter<'subgraphs, Built> {
        let mut config = self.config.expect("config is required");
        let subgraphs = self.subgraphs;
        let mut _hold_until_drop: Vec<Box<dyn Any>> = vec![];

        // change the supergraph to use the test subgraphs address
        if let Some(subgraphs) = subgraphs {
            let addr = subgraphs.addr();
            match &config.supergraph {
                hive_router_config::supergraph::SupergraphSource::File { path, .. } => {
                    let path = path.as_ref().expect("supergraph file path is required");

                    let original = std::fs::read_to_string(&path.absolute)
                        .expect("failed to read supergraph file");
                    let with_subgraphs_addr =
                        original.replace("0.0.0.0:4200", addr.to_string().as_str());

                    let temp_file = NamedTempFile::with_suffix(".graphql")
                        .expect("failed to create temp supergraph file");
                    std::fs::write(temp_file.path(), with_subgraphs_addr)
                        .expect("failed to write temp supergraph file");

                    let temp_path = hive_router_config::primitives::file_path::FilePath {
                        relative: temp_file.path().to_str().unwrap().to_string(),
                        absolute: temp_file.path().to_str().unwrap().to_string(),
                    };
                    let temp_absolute_path = temp_path.absolute.clone();

                    // close the file handle but keep the path for cleanup on drop
                    // useful when running many tests in parallel to avoid hitting the open file limit
                    let temp_path_handle = temp_file.into_temp_path();

                    config.supergraph = hive_router_config::supergraph::SupergraphSource::File {
                        path: Some(temp_path),
                        // TODO: we disable polling, but what if it was enabled?
                        poll_interval: None,
                    };

                    _hold_until_drop.push(Box::new(temp_path_handle));

                    info!(
                        "Using supergraph at {} to use test subgraphs with address {}",
                        temp_absolute_path, addr
                    );
                }
                _ => warn!("Only file-based supergraph sources are supported in tests"),
            }
        }

        TestRouter {
            graphql_path: config.graphql_path().to_string(),
            websocket_path: config.websocket_path().map(|p| p.to_string()),
            config: Some(config),
            subgraphs,
            handle: None,
            _hold_until_drop,
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
        let permit = Arc::clone(&CONCURRENCY_SEMAPHORE)
            .acquire_owned()
            .await
            .expect("concurrency semaphore closed");

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

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match serv.get("/health").send().await {
                    Ok(response) => {
                        if response.status() == 200 {
                            break;
                        }
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        })
        .await
        .expect("/health did not return 200 within 3 seconds");

        info!("Waiting for readiness check to pass...");

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match serv.get("/readiness").send().await {
                    Ok(response) => {
                        if response.status() == 200 {
                            break;
                        }
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        })
        .await
        .expect("/readiness did not return 200 within 3 seconds");

        let mut hold_until_drop = self._hold_until_drop;
        hold_until_drop.push(Box::new(subscription_guard));
        hold_until_drop.push(Box::new(permit));

        TestRouter {
            graphql_path: self.graphql_path,
            websocket_path: self.websocket_path,
            handle: Some(TestRouterHandle {
                serv,
                bg_tasks_manager,
            }),
            config: self.config,
            subgraphs: self.subgraphs,
            _hold_until_drop: hold_until_drop,
            _state: PhantomData,
        }
    }
}

impl<'subgraphs> TestRouter<'subgraphs, Started> {
    pub async fn send_graphql_request(
        &self,
        query: &str,
        variables: Option<sonic_rs::Value>,
        headers: Option<http::HeaderMap>,
    ) -> ClientResponse {
        let mut req = self
            .handle
            .as_ref()
            .unwrap()
            .serv
            .post(self.graphql_path.as_str())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/graphql-response+json");

        if let Some(headers) = headers {
            for (key, value) in headers.iter() {
                req = req.set_header(key, value);
            }
        }

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
