use async_trait::async_trait;
use ntex::rt::{spawn, JoinHandle};
use std::future::Future;
pub use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::telemetry::logging::targets;

#[async_trait]
pub trait BackgroundTask: Send + Sync {
    fn id(&self) -> &str;
    async fn run(&self, token: CancellationToken);
}

pub struct BackgroundTasksManager {
    cancellation_token: CancellationToken,
    handles: Vec<JoinHandle<()>>,
    graceful_handles: Vec<JoinHandle<()>>,
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
            graceful_handles: Vec::new(),
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

    /// Registers a task whose cancellation cleanup must finish before router shutdown completes.
    pub fn register_graceful_task<T>(&mut self, task: T)
    where
        T: BackgroundTask + 'static,
    {
        info!(
            target: targets::CORE,
            task_id = task.id(),
            "registering background task"
        );

        let child_token = self.cancellation_token.clone();
        self.graceful_handles.push(spawn(async move {
            task.run(child_token).await;
        }));
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
        for handle in self
            .handles
            .drain(..)
            .chain(self.graceful_handles.drain(..))
        {
            handle.cancel();
        }
        info!(target: targets::CORE, "all background tasks have been shut down.");
    }

    /// Stops ordinary tasks immediately and waits for graceful task cleanup to finish.
    pub async fn graceful_shutdown(&mut self) {
        debug!(
            target: targets::CORE,
            "shutdown triggered, stopping all background tasks..."
        );
        self.cancellation_token.cancel();
        for handle in self.handles.drain(..) {
            handle.cancel();
        }
        for handle in self.graceful_handles.drain(..) {
            let _ = handle.await;
        }
        info!(
            target: targets::CORE,
            "all background tasks have been shut down gracefully."
        );
    }
}
