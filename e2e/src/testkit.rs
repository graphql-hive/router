use std::{any::Any, collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use bollard::{
    exec::{CreateExecOptions, StartExecResults},
    query_parameters::CreateImageOptionsBuilder,
    secret::{ContainerCreateBody, ContainerCreateResponse, CreateImageInfo, HostConfig, PortMap},
    Docker,
};
use futures_util::TryStreamExt;
use hive_router::{
    configure_app_from_config, configure_ntex_app, init_rustls_crypto_provider,
    invoke_shutdown_hooks, plugins::plugins_service::PluginService, telemetry::Telemetry, BoxError,
    PluginRegistry, RouterSharedState, SchemaState,
};
use hive_router_config::{load_config, parse_yaml_config, HiveRouterConfig};
use hive_router_internal::background_tasks::BackgroundTasksManager;
use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
use ntex::{
    http::Request,
    web::{
        self,
        test::{self, TestRequest},
        WebResponse,
    },
    Pipeline, Service,
};
use subgraphs::{start_subgraphs_server, RequestLog, SubgraphsServiceState};
use tracing::{info, warn};

pub mod otel;

pub fn init_graphql_request(
    query: &str,
    variables: Option<HashMap<String, sonic_rs::Value>>,
) -> TestRequest {
    let body = GraphQLParams {
        query: Some(query.to_string()),
        operation_name: None,
        variables: variables.unwrap_or_default(),
        extensions: None,
    };
    test::TestRequest::post().uri("/graphql").set_json(&body)
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
    subgraph_shared_state: Arc<SubgraphsServiceState>,
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

        loop {
            match reqwest::get(&subgraph_shared_state.health_check_url).await {
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
        self.subgraph_shared_state
            .request_log
            .get(&format!("/{}", subgraph_name))
            .map(|entry| entry.value().clone())
    }
}

pub async fn init_router_from_config_file(
    config_path: &str,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    BoxError,
> {
    let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(config_path);
    let router_config = load_config(Some(supergraph_path.to_str().unwrap().to_string()))?;

    _init_router_from_config(router_config, PluginRegistry::new()).await
}

pub async fn init_router_from_config_file_with_plugins(
    config_path: &str,
    plugin_registry: PluginRegistry,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    BoxError,
> {
    let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(config_path);
    let router_config = load_config(Some(supergraph_path.to_str().unwrap().to_string()))?;
    _init_router_from_config(router_config, plugin_registry).await
}

pub async fn init_router_from_config_inline(
    config_yaml: &str,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    BoxError,
> {
    _init_router_from_config_inline(config_yaml, PluginRegistry::new()).await
}

pub async fn init_router_from_config_inline_with_plugins(
    config_yaml: &str,
    plugin_registry: PluginRegistry,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    BoxError,
> {
    _init_router_from_config_inline(config_yaml, plugin_registry).await
}

async fn _init_router_from_config_inline(
    config_yaml: &str,
    plugin_registry: PluginRegistry,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    BoxError,
> {
    let router_config = parse_yaml_config(config_yaml.to_string())?;
    _init_router_from_config(router_config, plugin_registry).await
}

pub struct TestRouterApp<T> {
    pub app: Pipeline<T>,
    pub shared_state: Arc<RouterSharedState>,
    pub schema_state: Arc<SchemaState>,
    pub bg_tasks_manager: BackgroundTasksManager,
    pub telemetry: Telemetry,
    // List of dependencies to hold until shutdown of the router
    pub _dependencies: Vec<Box<dyn Any>>,
}

impl<S> TestRouterApp<S> {
    pub async fn call<R>(&self, req: R) -> Result<S::Response, S::Error>
    where
        S: Service<R>,
    {
        self.app.call(req).await
    }

    pub async fn shutdown(&self) {
        self.schema_state.normalize_cache.run_pending_tasks().await;
        self.schema_state.plan_cache.run_pending_tasks().await;
        self.schema_state.validate_cache.run_pending_tasks().await;
        invoke_shutdown_hooks(&self.shared_state).await;
    }

    pub fn hold_until_shutdown(&mut self, dependency: Box<dyn Any>) {
        self._dependencies.push(dependency);
    }
}

async fn _init_router_from_config(
    router_config: HiveRouterConfig,
    plugin_registry: PluginRegistry,
) -> Result<
    TestRouterApp<
        impl ntex::Service<ntex::http::Request, Response = WebResponse, Error = ntex::web::Error>,
    >,
    BoxError,
> {
    init_rustls_crypto_provider();
    let (telemetry, subscriber) = Telemetry::init_subscriber(&router_config)?;
    let subscription_guard = tracing::subscriber::set_default(subscriber);

    let mut bg_tasks_manager = BackgroundTasksManager::new();
    let http_config = router_config.http.clone();
    let graphql_path = http_config.graphql_endpoint();
    let (shared_state, schema_state) = configure_app_from_config(
        router_config,
        telemetry.context.clone(),
        &mut bg_tasks_manager,
        plugin_registry,
    )
    .await?;

    let ntex_app = test::init_service(
        web::App::new()
            .wrap(PluginService)
            .state(shared_state.clone())
            .state(schema_state.clone())
            .configure(|m| configure_ntex_app(m, graphql_path)),
    )
    .await;

    Ok(TestRouterApp {
        app: ntex_app,
        shared_state,
        schema_state,
        bg_tasks_manager,
        telemetry,
        _dependencies: vec![Box::new(subscription_guard)],
    })
}

/// A guard that sets an environment variable to a specified value upon creation
/// and restores its original value (or removes it) when dropped.
pub struct EnvVarGuard {
    key: String,
    original_value: Option<String>,
}

impl EnvVarGuard {
    pub fn new(key: &str, value: &str) -> Self {
        let original_value = std::env::var(key).ok();
        // Because environment variables are global state, and modifying them can cause data races if multiple threads accesses them simultaneously,
        // we use unsafe block to indicate that we are aware of the potential risks.
        // In the e2e tests here, it's acceptable as tests are run one after another and we control the environment.
        unsafe {
            std::env::set_var(key, value);
        }

        EnvVarGuard {
            key: key.to_string(),
            original_value,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.original_value {
                Some(value) => std::env::set_var(&self.key, value),
                None => std::env::remove_var(&self.key),
            }
        }
    }
}

impl<T> Drop for TestRouterApp<T> {
    fn drop(&mut self) {
        self.bg_tasks_manager.shutdown();
        shutdown_telemetry_sync(&self.telemetry);
    }
}

fn shutdown_telemetry_sync(telemetry: &Telemetry) {
    let Some(provider) = telemetry.provider.clone() else {
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
        if let StartExecResults::Attached { mut output, .. } =
            self.docker.start_exec(&exec.id, None).await?
        {
            while let Some(msg) = output.try_next().await? {
                print!("{}", msg);
            }
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
