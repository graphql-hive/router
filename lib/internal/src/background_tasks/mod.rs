use async_trait::async_trait;
use std::future::Future;
pub use tokio_util::sync::CancellationToken;
use tracing::info;

#[async_trait]
pub trait BackgroundTask: Send + Sync {
    fn id(&self) -> &str;
    async fn run(&self, token: CancellationToken);
}

pub struct BackgroundTasksManager {
    cancellation_token: CancellationToken,
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
        }
    }

    pub fn register_task<T>(&mut self, task: T)
    where
        T: BackgroundTask + 'static,
    {
        info!("registering background task: {}", task.id());
        let child_token = self.cancellation_token.clone();
        ntex_rt::spawn(async move {
            task.run(child_token).await;
        });
    }

    pub fn register_handle<F>(&mut self, f: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        ntex_rt::spawn(f);
    }

    pub fn shutdown(&self) {
        info!("shutdown triggered, stopping all background tasks...");
        self.cancellation_token.cancel();
        info!("all background tasks have been shut down gracefully.");
    }
}
