use async_trait::async_trait;
use ntex::rt::Arbiter;
use std::{future::Future, sync::Arc};
use tokio_util::sync::CancellationToken;
use tracing::info;

#[async_trait]
pub trait BackgroundTask: Send + Sync {
    fn id(&self) -> &str;
    async fn run(&self, token: CancellationToken);
}

pub struct BackgroundTasksManager {
    cancellation_token: CancellationToken,
    arbiter: Arbiter,
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
            arbiter: Arbiter::new(),
        }
    }

    pub fn register_task<T>(&mut self, task: Arc<T>)
    where
        T: BackgroundTask + 'static,
    {
        info!("registering background task: {}", task.id());
        let child_token = self.cancellation_token.clone();

        self.arbiter.spawn(async move {
            task.run(child_token).await;
        });
    }

    pub fn register_handle<F>(&mut self, f: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.arbiter.spawn(f);
    }

    pub async fn shutdown(self) {
        info!("shutdown triggered, stopping all background tasks...");

        self.cancellation_token.cancel();
        self.arbiter.stop();

        info!("all background tasks have been shut down gracefully.");
    }
}
