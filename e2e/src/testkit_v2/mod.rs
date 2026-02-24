pub mod otel;

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
use tempfile::{NamedTempFile, TempPath};
use tokio::{
    net::TcpListener,
    sync::{oneshot, Semaphore},
};
use tracing::{info, warn};

use hive_router::{
    background_tasks::BackgroundTasksManager, configure_app_from_config, configure_ntex_app,
    init_rustls_crypto_provider, telemetry::Telemetry, SchemaState,
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

lazy_static! {
    /// Ensures only one `EnvVarsGuard` exists at a time, preventing concurrent mutation of
    /// environment variables (which are global process state and not thread-safe to modify).
    static ref ENV_VAR_SEMAPHORE: Arc<Semaphore> = Arc::new(Semaphore::new(1));
}

/// A guard that sets one or more environment variables and restores their original values (or
/// removes them) when dropped. Only one instance may exist at a time across all threads;
/// `apply()` is async and blocks until any previous guard has been dropped.
///
/// Usage: `EnvVarsGuard::new().set("key", "value").set("key2", "value2").apply().await`
pub struct EnvVarsGuard {
    pending: Vec<(String, String)>,
    vars: Vec<(String, Option<String>)>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl EnvVarsGuard {
    pub fn new() -> Self {
        EnvVarsGuard {
            pending: vec![],
            vars: vec![],
            permit: None,
        }
    }

    pub fn set(mut self, key: &str, value: &str) -> Self {
        self.pending.push((key.to_string(), value.to_string()));
        self
    }

    /// Applies the pending environment variable changes, returning a guard that
    /// will restore them on drop. This method is async and will block until any
    /// previous guard has been dropped to ensure that environment variable mutations
    /// are not done concurrently.
    pub async fn apply(mut self) -> Self {
        self.permit = Some(
            Arc::clone(&ENV_VAR_SEMAPHORE)
                .acquire_owned()
                .await
                .expect("env var semaphore closed"),
        );

        self.vars = self
            .pending
            .iter()
            .map(|(key, value)| {
                let original = std::env::var(key).ok();
                // SAFETY: environment variables are global state; we serialise all mutations
                // through ENV_VAR_SEMAPHORE so only one guard can set/restore vars at a time.
                unsafe { std::env::set_var(key, value) };
                (key.to_string(), original)
            })
            .collect();

        self
    }
}

impl Drop for EnvVarsGuard {
    fn drop(&mut self) {
        for (key, original) in &self.vars {
            // SAFETY: same as in `apply`; the permit is still held here and released after drop.
            unsafe {
                match original {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

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

    /// Creates a temporary supergraph file with the content of the given file but with the subgraphs
    /// address replaced with the test subgraphs address.
    ///
    /// The temp file will be automatically deleted when the returned TempPath is dropped.
    pub fn supergraph_temp_file_with_addr(&self, supergraph_file: &str) -> TempPath {
        let original =
            std::fs::read_to_string(supergraph_file).expect("failed to read supergraph file");
        let with_addr = self.supergraph_with_addr(original);

        let temp_file =
            NamedTempFile::with_suffix(".graphql").expect("failed to create temp supergraph file");
        std::fs::write(temp_file.path(), with_addr).expect("failed to write temp supergraph file");

        // close the file handle but keep the path for cleanup on drop
        // useful when running many tests in parallel to avoid hitting the open file limit
        let temp_path = temp_file.into_temp_path();

        info!(
            "Using supergraph at {} to use test subgraphs with address {}",
            temp_path
                .to_str()
                .expect("failed to convert temp path to string"),
            self.addr()
        );

        temp_path
    }

    /// Replaces the subgraphs address in the given supergraph string with the test
    /// subgraphs address and returns the modified supergraph.
    ///
    /// It will replace all occurrences of `0.0.0.0:4200` with the test subgraphs address.
    pub fn supergraph_with_addr(&self, supergraph: impl Into<String>) -> String {
        let original: String = supergraph.into();
        original.replace("0.0.0.0:4200", self.addr().to_string().as_str())
    }
}

impl Drop for TestSubgraphsHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.take().map(|tx| tx.send(()));
    }
}

// router

pub struct TestRouterBuilder<'subgraphs> {
    wait_for_healthy_on_start: bool,
    wait_for_ready_on_start: bool,
    config: Option<HiveRouterConfig>,
    subgraphs: Option<&'subgraphs TestSubgraphs<Started>>,
}

impl<'subgraphs> TestRouterBuilder<'subgraphs> {
    pub fn new() -> Self {
        Self {
            wait_for_healthy_on_start: true,
            wait_for_ready_on_start: true,
            config: None,
            subgraphs: None,
        }
    }

    pub fn inline_config(mut self, config_yaml: impl Into<String>) -> Self {
        let router_config = parse_yaml_config(config_yaml.into()).unwrap();
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

    pub fn skip_wait_for_healthy_on_start(mut self) -> Self {
        self.wait_for_healthy_on_start = false;
        self
    }

    pub fn skip_wait_for_ready_on_start(mut self) -> Self {
        self.wait_for_ready_on_start = false;
        self
    }

    pub fn build(self) -> TestRouter<'subgraphs, Built> {
        let mut config = self.config.expect("config is required");
        let subgraphs = self.subgraphs;
        let mut _hold_until_drop: Vec<Box<dyn Any>> = vec![];

        // change the supergraph to use the test subgraphs address
        if let Some(subgraphs) = subgraphs {
            match &config.supergraph {
                hive_router_config::supergraph::SupergraphSource::File { path, .. } => {
                    let supergraph_path = path.as_ref().expect("supergraph file path is required");

                    let temp_path =
                        subgraphs.supergraph_temp_file_with_addr(supergraph_path.absolute.as_str());

                    let supergraph_file_path =
                        hive_router_config::primitives::file_path::FilePath {
                            relative: temp_path.to_str().unwrap().to_string(),
                            absolute: temp_path.to_str().unwrap().to_string(),
                        };

                    config.supergraph = hive_router_config::supergraph::SupergraphSource::File {
                        path: Some(supergraph_file_path),
                        // TODO: we disable polling, but what if it was enabled?
                        poll_interval: None,
                    };

                    _hold_until_drop.push(Box::new(temp_path));
                }
                _ => warn!("Only file-based supergraph sources are supported in tests"),
            }
        }

        TestRouter {
            wait_for_healthy_on_start: self.wait_for_healthy_on_start,
            wait_for_ready_on_start: self.wait_for_ready_on_start,
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
    schema_state: Arc<SchemaState>,
    serv: test::TestServer,
    bg_tasks_manager: BackgroundTasksManager,
    telemetry: Telemetry,
}

impl Drop for TestRouterHandle {
    fn drop(&mut self) {
        // shut down backgroun tasks
        self.bg_tasks_manager.shutdown();

        // shut down telemetry
        let Some(provider) = self.telemetry.provider.clone() else {
            return;
        };
        let dispatch = tracing::dispatcher::get_default(|current| current.clone());
        let handle = std::thread::spawn(move || {
            tracing::dispatcher::with_default(&dispatch, || {
                tracing::info!(
                    component = "telemetry",
                    layer = "provider",
                    "shutdown scheduled"
                );
                let _ = provider.force_flush();
                let _ = provider.shutdown();
                tracing::info!(
                    component = "telemetry",
                    layer = "provider",
                    "shutdown completed"
                );
            });
        });
        let _ = handle.join();
    }
}

pub struct TestRouter<'subgraphs, State> {
    wait_for_healthy_on_start: bool,
    wait_for_ready_on_start: bool,
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

        // capture the current tracing dispatch so it can be propagated to the
        // server thread spawned by test::server (which runs on a separate thread
        // and would otherwise use the no-op global subscriber)
        let serv_dispatch = tracing::dispatcher::get_default(|d| d.clone());

        let serv_schema_state = schema_state.clone();
        let serv_graphql_path = self.graphql_path.clone();
        let serv_websocket_path = self.websocket_path.clone();
        let serv = test::server(move || {
            let shared_state = shared_state.clone();
            let schema_state = serv_schema_state.clone();
            let serv_graphql_path = serv_graphql_path.clone();
            let serv_websocket_path = serv_websocket_path.clone();

            // set the tracing dispatch on the server thread. the guard is
            // intentionally leaked: dropping it would restore the no-op default
            // dispatch, undoing what we just set. the guard is `!send` (thread-
            // local), so we can't move it back to the test thread. this is fine
            // because when the server thread exits (on testserver drop) the
            // thread-local storage is reclaimed by the os, and there is no prior
            // dispatch to restore
            let guard = tracing::dispatcher::set_default(&serv_dispatch);
            std::mem::forget(guard);

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

        let mut hold_until_drop = self._hold_until_drop;
        hold_until_drop.push(Box::new(subscription_guard));
        let started = TestRouter {
            wait_for_healthy_on_start: self.wait_for_healthy_on_start,
            wait_for_ready_on_start: self.wait_for_ready_on_start,
            graphql_path: self.graphql_path,
            websocket_path: self.websocket_path,
            handle: Some(TestRouterHandle {
                schema_state,
                serv,
                bg_tasks_manager,
                telemetry,
            }),
            config: self.config,
            subgraphs: self.subgraphs,
            _hold_until_drop: hold_until_drop,
            _state: PhantomData,
        };

        if self.wait_for_healthy_on_start {
            info!("Waiting for healthcheck to pass...");
            started.wait_for_healthy(None).await;
        }

        if self.wait_for_ready_on_start {
            info!("Waiting for readiness check to pass...");
            started.wait_for_ready(None).await;
        }

        started
    }
}

impl<'subgraphs> TestRouter<'subgraphs, Started> {
    pub fn schema_state(&self) -> &Arc<SchemaState> {
        &self.handle.as_ref().unwrap().schema_state
    }

    pub async fn flush_internal_cache(&self) {
        self.schema_state()
            .normalize_cache
            .run_pending_tasks()
            .await;
        self.schema_state().plan_cache.run_pending_tasks().await;
        self.schema_state().validate_cache.run_pending_tasks().await;
    }

    pub fn serv(&self) -> &test::TestServer {
        &self.handle.as_ref().unwrap().serv
    }

    /// Waits for the /health endpoint to return 200 OK, with an optional timeout (defaults to 5 seconds).
    pub async fn wait_for_healthy(&self, timeout: Option<Duration>) {
        tokio::time::timeout(timeout.unwrap_or(Duration::from_secs(5)), async {
            loop {
                match self.serv().get("/health").send().await {
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
        .expect("healthcheck timed out");
    }

    /// Waits for the /readiness endpoint to return 200 OK, with an optional timeout (defaults to 5 seconds).
    pub async fn wait_for_ready(&self, timeout: Option<Duration>) {
        tokio::time::timeout(timeout.unwrap_or(Duration::from_secs(5)), async {
            loop {
                match self.serv().get("/readiness").send().await {
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
        .expect("readiness timed out");
    }

    pub fn graphql_path(&self) -> &str {
        &self.graphql_path
    }

    pub async fn send_graphql_request(
        &self,
        query: &str,
        variables: Option<sonic_rs::Value>,
        headers: Option<http::HeaderMap>,
    ) -> ClientResponse {
        let mut req = self
            .serv()
            .post(self.graphql_path())
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
