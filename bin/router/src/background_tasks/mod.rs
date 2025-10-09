use async_trait::async_trait;
use futures::future::join_all;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

#[async_trait]
pub trait BackgroundTask: Send + Sync {
    fn id(&self) -> &str;
    async fn run(&self, token: CancellationToken);
}

pub struct BackgroundTasksManager {
    cancellation_token: CancellationToken,
    task_handles: Vec<JoinHandle<()>>,
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
            task_handles: Vec::new(),
        }
    }

    pub fn register_task<T>(&mut self, task: Arc<T>)
    where
        T: BackgroundTask + 'static,
    {
        info!("registering background task: {}", task.id());
        let child_token = self.cancellation_token.clone();

        let handle = tokio::spawn(async move {
            task.run(child_token).await;
        });

        self.task_handles.push(handle);
    }

    pub async fn shutdown(self) {
        info!("shutdown triggered, stopping all background tasks...");
        self.cancellation_token.cancel();

        debug!("waiting for background tasks to finish...");
        join_all(self.task_handles).await;

        println!("all background tasks have been shut down gracefully.");
    }
}
