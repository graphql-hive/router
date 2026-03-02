use bollard::{
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
    models::{ContainerCreateBody, HostConfig, PortBinding},
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, RemoveContainerOptionsBuilder,
    },
    Docker,
};
use futures_util::TryStreamExt;
use std::{collections::HashMap, marker::PhantomData};

use super::{Built, Started};

pub struct TestDockerContainerBuilder {
    name: String,
    image: String,
    ports: HashMap<u16, u16>,
    env: Vec<String>,
}

impl TestDockerContainerBuilder {
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
            ports: HashMap::new(),
            env: vec![],
        }
    }

    pub fn port(mut self, container_port: u16, host_port: u16) -> Self {
        self.ports.insert(container_port, host_port);
        self
    }

    pub fn env(mut self, env: impl Into<String>) -> Self {
        self.env.push(env.into());
        self
    }

    pub fn build(self) -> TestDockerContainer<Built> {
        TestDockerContainer {
            name: self.name,
            image: self.image,
            ports: self.ports,
            env: self.env,
            handle: None,
            _state: PhantomData,
        }
    }
}

struct TestDockerContainerHandle {
    name: String,
    docker: Docker,
}

impl Drop for TestDockerContainerHandle {
    fn drop(&mut self) {
        let docker = self.docker.clone();
        let name = self.name.clone();
        let _ = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build shutdown runtime")
                .block_on(async move {
                    let _ = docker
                        .remove_container(
                            &name,
                            Some(RemoveContainerOptionsBuilder::new().force(true).build()),
                        )
                        .await;
                })
        })
        .join();
    }
}

pub struct TestDockerContainer<State> {
    name: String,
    image: String,
    ports: HashMap<u16, u16>,
    env: Vec<String>,
    handle: Option<TestDockerContainerHandle>,
    _state: PhantomData<State>,
}

impl TestDockerContainer<Built> {
    pub fn builder(
        name: impl Into<String>,
        image: impl Into<String>,
    ) -> TestDockerContainerBuilder {
        TestDockerContainerBuilder::new(name, image)
    }

    pub async fn start(self) -> TestDockerContainer<Started> {
        let docker =
            Docker::connect_with_local_defaults().expect("failed to connect to docker daemon");

        docker
            .create_image(
                Some(
                    CreateImageOptionsBuilder::default()
                        .from_image(&self.image)
                        .build(),
                ),
                None,
                None,
            )
            .try_collect::<Vec<_>>()
            .await
            .expect("failed to pull docker image");

        let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        for (container_port, host_port) in &self.ports {
            port_bindings.insert(
                format!("{}/tcp", container_port),
                Some(vec![PortBinding {
                    host_ip: Some("127.0.0.1".to_string()),
                    host_port: Some(host_port.to_string()),
                }]),
            );
        }

        let host_config = HostConfig {
            port_bindings: Some(port_bindings),
            ..Default::default()
        };

        let _ = docker
            .remove_container(
                &self.name,
                Some(RemoveContainerOptionsBuilder::new().force(true).build()),
            )
            .await;

        docker
            .create_container(
                Some(
                    CreateContainerOptionsBuilder::new()
                        .name(&self.name)
                        .build(),
                ),
                ContainerCreateBody {
                    image: Some(self.image.clone()),
                    env: if self.env.is_empty() {
                        None
                    } else {
                        Some(self.env.clone())
                    },
                    host_config: Some(host_config),
                    ..Default::default()
                },
            )
            .await
            .expect("failed to create docker container");

        docker
            .start_container(&self.name, None)
            .await
            .expect("failed to start docker container");

        TestDockerContainer {
            name: self.name.clone(),
            image: self.image,
            ports: self.ports,
            env: self.env,
            handle: Some(TestDockerContainerHandle {
                name: self.name,
                docker,
            }),
            _state: PhantomData,
        }
    }
}

impl TestDockerContainer<Started> {
    pub async fn exec(&self, cmd: Vec<&str>) {
        let handle = self.handle.as_ref().expect("container not started");

        let exec = handle
            .docker
            .create_exec(
                &handle.name,
                CreateExecOptions {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(cmd),
                    ..Default::default()
                },
            )
            .await
            .expect("failed to create exec");

        let results = handle
            .docker
            .start_exec(&exec.id, None::<StartExecOptions>)
            .await
            .expect("failed to start exec");

        if let StartExecResults::Attached { mut output, .. } = results {
            while output
                .try_next()
                .await
                .expect("exec output error")
                .is_some()
            {}
        }
    }
}
