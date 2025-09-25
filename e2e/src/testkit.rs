use std::{path::PathBuf, sync::Once};

use hive_router::{
    background_tasks::BackgroundTasksManager, configure_app_from_config, configure_ntex_app,
};
use hive_router_config::load_config;
use lazy_static::lazy_static;
use ntex::{
    web::{self, test, test::TestRequest, WebResponse},
    Pipeline,
};
use sonic_rs::json;
use subgraphs::{start_subgraphs_server, RequestLog, SubgraphsServiceState};
use tracing::subscriber::set_global_default;
use tracing_subscriber::{
    fmt::{self},
    layer::SubscriberExt,
    EnvFilter,
};

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

lazy_static! {
    static ref TRACING_INIT: Once = Once::new();
}

#[allow(dead_code)] // call this at the beginning of the test if you wish to see gw logs
pub fn init_logger() {
    TRACING_INIT.call_once(|| {
        let subscriber = tracing_subscriber::registry()
            .with(fmt::layer().with_test_writer())
            .with(EnvFilter::from_default_env());

        let _ = set_global_default(subscriber);
    });
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
    pub fn start() -> Self {
        let (_server_handle, shutdown_tx, subgraph_shared_state) = start_subgraphs_server(None);

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
    Pipeline<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    Box<dyn std::error::Error>,
> {
    // init_logger();

    let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(config_path);
    let router_config = load_config(Some(supergraph_path.to_str().unwrap().to_string()))?;
    let mut bg_tasks_manager = BackgroundTasksManager::new();
    let shared_state = configure_app_from_config(router_config, &mut bg_tasks_manager).await?;

    let ntex_app = test::init_service(
        web::App::new()
            .state(shared_state.clone())
            .configure(configure_ntex_app),
    )
    .await;

    Ok(ntex_app)
}
