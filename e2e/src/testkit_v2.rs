use std::{marker::PhantomData, sync::Arc, time::Duration};

use hive_router::{
    background_tasks::BackgroundTasksManager, configure_app_from_config, configure_ntex_app,
    init_rustls_crypto_provider,
};
use hive_router_config::{parse_yaml_config, HiveRouterConfig};
use ntex::{
    http::client::ClientResponse,
    web::{self, test},
};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use sonic_rs::json;
use subgraphs::{start_subgraphs_server, RequestLog, SubgraphsServiceState, SubscriptionProtocol};
use tokio::sync::oneshot::Sender;
use tracing::{info, warn};

pub struct Built;
pub struct Started;

pub struct TestRouterBuilder {
    config: Option<HiveRouterConfig>,
    start_subgraphs: bool,
}

impl TestRouterBuilder {
    pub fn new() -> Self {
        Self {
            config: None,
            start_subgraphs: false,
        }
    }

    pub fn inline_config(mut self, config_yaml: &str) -> Self {
        let router_config = parse_yaml_config(config_yaml.to_string()).unwrap();
        self.config = Some(router_config);
        self
    }

    pub fn with_subgraphs(mut self) -> Self {
        self.start_subgraphs = true;
        self
    }

    pub fn build(self) -> TestRouter<Built> {
        TestRouter {
            config: Some(self.config.expect("config is required")),
            start_subgraphs: self.start_subgraphs,
            handle: None,
            _state: PhantomData,
        }
    }
}

impl Default for TestRouterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

struct SubgraphsHandle {
    shutdown_tx: Option<Sender<()>>,
    #[allow(dead_code)]
    state: Arc<SubgraphsServiceState>,
}

impl SubgraphsHandle {
    async fn start() -> Self {
        let (_server_handle, shutdown_tx, state) = start_subgraphs_server(
            // TODO: should auto-allocate free port
            Some(4200),
            // TODO: make configurable
            SubscriptionProtocol::Auto,
            None,
        );

        loop {
            match reqwest::get(&state.health_check_url).await {
                Ok(response) if response.status().is_success() => {
                    break;
                }
                _ => {
                    warn!("Subgraphs not healthy yet, retrying in 50ms");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }

        Self {
            shutdown_tx: Some(shutdown_tx),
            state,
        }
    }
}

impl Drop for SubgraphsHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

struct TestRouterHandle {
    serv: test::TestServer,
    bg_tasks_manager: BackgroundTasksManager,
    #[allow(dead_code)]
    subgraphs: Option<SubgraphsHandle>,
}

impl Drop for TestRouterHandle {
    fn drop(&mut self) {
        self.bg_tasks_manager.shutdown();
    }
}

pub struct TestRouter<State> {
    config: Option<HiveRouterConfig>,
    start_subgraphs: bool,
    handle: Option<TestRouterHandle>,
    _state: PhantomData<State>,
}

impl TestRouter<Built> {
    pub async fn start(mut self) -> Result<TestRouter<Started>, Box<dyn std::error::Error>> {
        init_rustls_crypto_provider();

        let subgraphs = if self.start_subgraphs {
            Some(SubgraphsHandle::start().await)
        } else {
            None
        };

        let mut bg_tasks_manager = BackgroundTasksManager::new();
        let config = self.config.take().unwrap();
        let http_config = config.http.clone();
        let graphql_path = http_config.graphql_endpoint().to_string();

        let (shared_state, schema_state) =
            configure_app_from_config(config, &mut bg_tasks_manager).await?;

        let serv = test::server(move || {
            web::App::new()
                .state(shared_state.clone())
                .state(schema_state.clone())
                .configure(|m| configure_ntex_app(m, &graphql_path))
        });

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

        Ok(TestRouter {
            handle: Some(TestRouterHandle {
                serv,
                bg_tasks_manager,
                subgraphs,
            }),
            config: None,
            start_subgraphs: false,
            _state: PhantomData,
        })
    }
}

impl TestRouter<Started> {
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
            .post("/graphql")
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/graphql-response+json");

        req.send_json(&json!({
          "query": query,
          "variables": variables,
        }))
        .await
        .expect("Failed to send graphql request")
    }

    #[allow(dead_code)]
    pub async fn get_subgraph_requests_log(&self, subgraph_name: &str) -> Option<Vec<RequestLog>> {
        self.handle.as_ref().and_then(|h| {
            h.subgraphs.as_ref().and_then(|s| {
                s.state
                    .request_log
                    .get(&format!("/{}", subgraph_name))
                    .map(|entry| entry.value().clone())
            })
        })
    }
}
