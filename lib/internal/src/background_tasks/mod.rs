use async_trait::async_trait;
use futures::{stream::FuturesUnordered, StreamExt};
use ntex::rt::{spawn, JoinHandle};
use std::{future::Future, pin::Pin};
use tokio::sync::mpsc;
pub use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::telemetry::logging::targets;

#[async_trait]
pub trait BackgroundTask: Send + Sync {
    fn id(&self) -> &str;
    async fn run(&self, token: CancellationToken);
}

type DynamicTaskRegistration = (Box<dyn BackgroundTask>, CancellationToken);

/// Registers runtime-owned work with one router-managed background task.
#[derive(Clone)]
pub struct DynamicBackgroundTaskRegistrar {
    sender: mpsc::UnboundedSender<DynamicTaskRegistration>,
}

impl DynamicBackgroundTaskRegistrar {
    pub fn register_task<T>(&self, task: T, lifetime: CancellationToken)
    where
        T: BackgroundTask + 'static,
    {
        self.sender.send((Box::new(task), lifetime)).ok();
    }
}

struct DynamicBackgroundTasks {
    registrations: tokio::sync::Mutex<mpsc::UnboundedReceiver<DynamicTaskRegistration>>,
}

impl DynamicBackgroundTasks {
    fn new() -> (DynamicBackgroundTaskRegistrar, Self) {
        let (sender, registrations) = mpsc::unbounded_channel();
        (
            DynamicBackgroundTaskRegistrar { sender },
            Self {
                registrations: tokio::sync::Mutex::new(registrations),
            },
        )
    }
}

#[async_trait]
impl BackgroundTask for DynamicBackgroundTasks {
    fn id(&self) -> &str {
        "dynamic-background-tasks"
    }

    async fn run(&self, token: CancellationToken) {
        let mut registrations = self.registrations.lock().await;
        let mut tasks: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>> =
            FuturesUnordered::new();
        loop {
            tokio::select! {
                _ = token.cancelled() => return,
                registration = registrations.recv() => {
                    let Some((task, lifetime)) = registration else { return; };
                    let router_shutdown = token.clone();
                    tasks.push(Box::pin(async move {
                        let task_token = CancellationToken::new();
                        tokio::select! {
                            _ = router_shutdown.cancelled() => task_token.cancel(),
                            _ = lifetime.cancelled() => task_token.cancel(),
                            _ = task.run(task_token.clone()) => {},
                        }
                    }));
                }
                Some(()) = tasks.next(), if !tasks.is_empty() => {}
            }
        }
    }
}

pub struct BackgroundTasksManager {
    cancellation_token: CancellationToken,
    handles: Vec<JoinHandle<()>>,
}

impl Default for BackgroundTasksManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundTasksManager {
    pub fn new() -> Self {
        Self {
            cancellation_token: CancellationToken::new(),
            handles: Vec::new(),
        }
    }

    pub fn register_task<T>(&mut self, task: T)
    where
        T: BackgroundTask + 'static,
    {
        info!(
            target: targets::CORE,
            task_id = task.id(),
            "registering background task"
        );

        let child_token = self.cancellation_token.clone();
        let handle = spawn(async move {
            task.run(child_token).await;
        });
        self.handles.push(handle);
    }

    pub fn dynamic_registrar(&mut self) -> DynamicBackgroundTaskRegistrar {
        let (registrar, task) = DynamicBackgroundTasks::new();
        self.register_task(task);
        registrar
    }

    pub fn register_handle<F>(&mut self, f: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.handles.push(spawn(f));
    }

    pub fn shutdown(&mut self) {
        debug!(
            target: targets::CORE,
            "shutdown triggered, stopping all background tasks..."
        );
        self.cancellation_token.cancel();
        for handle in self.handles.drain(..) {
            handle.cancel();
        }
        info!(
            target: targets::CORE,
            "all background tasks have been shut down gracefully."
        );
    }
}
