use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use bollard::{
    exec::{CreateExecOptions, StartExecResults},
    query_parameters::CreateImageOptionsBuilder,
    secret::{ContainerCreateBody, ContainerCreateResponse, CreateImageInfo, HostConfig, PortMap},
    Docker,
};
use futures_util::TryStreamExt;
use hive_router::{
    background_tasks::BackgroundTasksManager, configure_app_from_config, configure_ntex_app,
    plugins::plugins_service::PluginService, PluginRegistry, RouterSharedState, SchemaState,
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
    plugin_registry: Option<PluginRegistry>,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    Box<dyn std::error::Error>,
> {
    let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(config_path);
    let router_config = load_config(Some(supergraph_path.to_str().unwrap().to_string()))?;

    init_router_from_config(router_config, plugin_registry).await
}

pub async fn init_router_from_config_inline(
    config_yaml: &str,
    plugin_registry: Option<PluginRegistry>,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    Box<dyn std::error::Error>,
> {
    let router_config = parse_yaml_config(config_yaml.to_string())?;
    init_router_from_config(router_config, plugin_registry).await
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
    plugin_registry: Option<PluginRegistry>,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    Box<dyn std::error::Error>,
> {
    let mut bg_tasks_manager = BackgroundTasksManager::new();
    let (shared_state, schema_state) =
        configure_app_from_config(router_config, &mut bg_tasks_manager, plugin_registry).await?;

    let ntex_app = test::init_service(
        web::App::new()
            .wrap(PluginService)
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

#[derive(Default)]
pub struct TestDockerContainerOpts {
    pub name: String,
    pub image: String,
    pub ports: HashMap<u16, u16>,
    pub env: Vec<String>,
}

pub struct TestDockerContainer {
    docker: Docker,
    container: ContainerCreateResponse,
}

impl TestDockerContainer {
    pub async fn async_new(opts: TestDockerContainerOpts) -> Result<Self, bollard::errors::Error> {
        let docker =
            Docker::connect_with_local_defaults().expect("Failed to connect to Docker daemon");
        let mut port_bindings = PortMap::new();
        for (container_port, host_port) in opts.ports.iter() {
            port_bindings.insert(
                format!("{}/tcp", container_port),
                Some(vec![bollard::models::PortBinding {
                    host_port: Some(host_port.to_string()),
                    ..Default::default()
                }]),
            );
        }
        let _: Vec<CreateImageInfo> = docker
            .create_image(
                Some(
                    CreateImageOptionsBuilder::default()
                        .from_image(&opts.image)
                        .build(),
                ),
                None,
                None,
            )
            .try_collect()
            .await
            .expect("Failed to pull the image");
        let container_exists = docker
            .list_containers(Some(bollard::query_parameters::ListContainersOptions {
                all: true,
                ..Default::default()
            }))
            .await?
            .into_iter()
            .any(|c| {
                c.names
                    .unwrap_or_default()
                    .iter()
                    .any(|name| name.trim_start_matches('/').eq(&opts.name))
            });
        if container_exists {
            docker
                .remove_container(
                    &opts.name,
                    Some(bollard::query_parameters::RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await
                .expect("Failed to remove existing container");
        }
        let container = docker
            .create_container(
                Some(
                    bollard::query_parameters::CreateContainerOptionsBuilder::default()
                        .name(&opts.name)
                        .build(),
                ),
                ContainerCreateBody {
                    image: Some(opts.image.to_string()),
                    host_config: Some(HostConfig {
                        port_bindings: Some(port_bindings),
                        ..Default::default()
                    }),
                    env: Some(opts.env),
                    ..Default::default()
                },
            )
            .await
            .expect("Failed to create the container");
        docker
            .start_container(
                &container.id,
                None::<bollard::query_parameters::StartContainerOptions>,
            )
            .await
            .expect("Failed to start the container");
        Ok(Self { docker, container })
    }
    pub async fn exec(&self, cmd: Vec<&str>) -> Result<(), bollard::errors::Error> {
        let exec = self
            .docker
            .create_exec(
                &self.container.id,
                CreateExecOptions {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(cmd),
                    ..Default::default()
                },
            )
            .await?;
        match self.docker.start_exec(&exec.id, None).await? {
            StartExecResults::Attached { mut output, .. } => {
                while let Some(msg) = output.try_next().await? {
                    print!("{}", msg);
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub async fn stop(&self) {
        self.docker
            .remove_container(
                &self.container.id,
                Some(bollard::query_parameters::RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .expect("Failed to remove the container");
    }
}
