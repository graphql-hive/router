use hive_router_config::jwt_auth::JwtAuthConfig;
use tokio_util::sync::CancellationToken;

use crate::background_tasks::BackgroundTask;

pub struct JwksProvidersManager {}

impl JwksProvidersManager {
    pub fn from_config(config: &JwtAuthConfig) -> Self {
        JwksProvidersManager {}
    }
}

#[async_trait::async_trait]
impl BackgroundTask for JwksProvidersManager {
    fn id(&self) -> &str {
        "jwt_auth"
    }

    async fn run(&self, token: CancellationToken) {
        // let mut interval = tokio::time::interval(Duration::from_secs(3));
        // loop {
        //     tokio::select! {
        //         _ = interval.tick() => { println!("running"); }
        //         _ = token.cancelled() => { println!("Shutting down."); return; }
        //     }
        // }
    }
}
