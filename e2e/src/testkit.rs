use std::{path::PathBuf, sync::Arc, time::Duration};

use hive_router::{
    background_tasks::BackgroundTasksManager, configure_app_from_config, configure_ntex_app,
    RouterSharedState, SchemaState,
};
use hive_router_config::{load_config, parse_yaml_config, HiveRouterConfig};
use ntex::{
    http::Request,
    web::{
        self,
        test::{self, TestRequest},
        WebResponse,
    },
    Pipeline, Service,
};
use sonic_rs::json;
use subgraphs::{start_subgraphs_server, RequestLog, SubgraphsServiceState};
use tracing::{info, warn};

pub fn init_graphql_request(op: &str, variables: Option<sonic_rs::Value>) -> TestRequest {
    let body = json!({
      "query": op,
      "variables": variables
    });

    test::TestRequest::post()
        .uri("/graphql")
        .header("content-type", "application/json")
        .set_payload(body.to_string())
}

pub async fn wait_for_readiness<S, E>(app: &Pipeline<S>)
where
    S: Service<Request, Response = WebResponse, Error = E>,
    E: std::fmt::Debug,
{
    info!("waiting for health check to pass...");

    loop {
        let req = test::TestRequest::get().uri("/health").to_request();

        match app.call(req).await {
            Ok(response) => {
                if response.status() == 200 {
                    break;
                }
            }
            Err(err) => {
                warn!("Server not healthy yet, retrying in 100ms: {:?}", err);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    info!("waiting for readiness check to pass...");

    loop {
        let req = test::TestRequest::get().uri("/readiness").to_request();

        match app.call(req).await {
            Ok(response) => {
                if response.status() == 200 {
                    break;
                }
            }
            Err(err) => {
                warn!("Server not ready, retrying in 100ms: {:?}", err);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

pub struct SubgraphsServer {
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    subgraph_shared_state: SubgraphsServiceState,
}

impl Drop for SubgraphsServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl SubgraphsServer {
    /// Port defaults to 4200 if None (as defined in supergraph.graphql)
    pub async fn start() -> Self {
        Self::start_with_port(4200).await
    }

    pub async fn start_with_port(port: u16) -> Self {
        let (_server_handle, shutdown_tx, subgraph_shared_state) =
            start_subgraphs_server(Some(port));

        let health_check_url = subgraph_shared_state.health_check_url.clone();
        loop {
            match reqwest::get(&health_check_url).await {
                Ok(response) if response.status().is_success() => {
                    // Server is up and running.
                    break;
                }
                _ => {
                    // Server not ready yet, wait and retry.
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        }

        Self {
            shutdown_tx: Some(shutdown_tx),
            subgraph_shared_state,
        }
    }

    pub async fn get_subgraph_requests_log(&self, subgraph_name: &str) -> Option<Vec<RequestLog>> {
        let log = self.subgraph_shared_state.request_log.lock().await;

        log.get(&format!("/{}", subgraph_name)).cloned()
    }
}

pub async fn init_router_from_config_file(
    config_path: &str,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    Box<dyn std::error::Error>,
> {
    let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(config_path);
    let router_config = load_config(Some(supergraph_path.to_str().unwrap().to_string()))?;

    init_router_from_config(router_config).await
}

pub async fn init_router_from_config_inline(
    config_yaml: &str,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    Box<dyn std::error::Error>,
> {
    let router_config = parse_yaml_config(config_yaml.to_string())?;
    init_router_from_config(router_config).await
}

pub struct TestRouterApp<T> {
    pub app: Pipeline<T>,
    #[allow(dead_code)]
    pub shared_state: Arc<RouterSharedState>,
    pub schema_state: Arc<SchemaState>,
    pub bg_tasks_manager: BackgroundTasksManager,
}

impl<S> TestRouterApp<S> {
    pub async fn call<R>(&self, req: R) -> Result<S::Response, S::Error>
    where
        S: Service<R>,
    {
        self.app.call(req).await
    }

    pub async fn flush_internal_cache(&self) {
        self.schema_state.normalize_cache.run_pending_tasks().await;
        self.schema_state.plan_cache.run_pending_tasks().await;
        self.schema_state.validate_cache.run_pending_tasks().await;
    }
}

pub async fn init_router_from_config(
    router_config: HiveRouterConfig,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    Box<dyn std::error::Error>,
> {
    let mut bg_tasks_manager = BackgroundTasksManager::new();
    let (shared_state, schema_state) =
        configure_app_from_config(router_config, &mut bg_tasks_manager).await?;

    let ntex_app = test::init_service(
        web::App::new()
            .state(shared_state.clone())
            .state(schema_state.clone())
            .configure(configure_ntex_app),
    )
    .await;

    Ok(TestRouterApp {
        app: ntex_app,
        shared_state,
        schema_state,
        bg_tasks_manager,
    })
}

impl<T> Drop for TestRouterApp<T> {
    fn drop(&mut self) {
        self.bg_tasks_manager.shutdown();
    }
}
