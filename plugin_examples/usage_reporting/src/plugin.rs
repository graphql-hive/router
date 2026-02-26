use std::{sync::Arc, time::Duration};

use hive_router::{
    async_trait,
    background_tasks::{BackgroundTask, CancellationToken},
    humantime_serde, ntex,
    plugins::{
        hooks::{
            on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    },
    tokio::sync::Mutex,
    tracing,
};
use serde::{Deserialize, Serialize};

/// A usage reporting plugin that sends everything on shutdown. Don't ask why? Testing...

#[derive(Serialize)]
struct UsageReport {
    query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_name: Option<String>,
}

pub struct UsageReportingPlugin {
    endpoint: String,
    reports: Arc<Mutex<Vec<UsageReport>>>,
}

#[derive(Deserialize)]
pub struct UsageReportingPluginConfig {
    endpoint: String,
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    interval: Duration,
}

#[async_trait]
impl RouterPlugin for UsageReportingPlugin {
    type Config = UsageReportingPluginConfig;
    fn plugin_name() -> &'static str {
        "usage_reporting"
    }
    fn on_plugin_init(mut payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let UsageReportingPluginConfig { endpoint, interval } = payload.config()?;
        let reports = Arc::new(Mutex::new(vec![]));
        payload.register_background_task(UsageReportingTask {
            endpoint: endpoint.clone(),
            reports: reports.clone(),
            interval,
        });
        payload.initialize_plugin(Self { endpoint, reports })
    }
    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        self.reports.lock().await.push(UsageReport {
            query: payload.operation_for_plan.to_string(),
            operation_name: payload.operation_for_plan.name.clone(),
        });
        tracing::trace!(
            "Pushed usage report for operation: {:?}",
            payload.operation_for_plan.name
        );
        payload.proceed()
    }
    async fn on_shutdown<'exec>(&'exec self) {
        tracing::trace!("Disposing UsageReportingPlugin");
        flush_reports(&self.endpoint, &self.reports).await;
    }
}

async fn flush_reports(endpoint: &str, reports: &Mutex<Vec<UsageReport>>) {
    let to_send = {
        let mut lock = reports.lock().await;
        std::mem::take(&mut *lock)
    };

    if to_send.is_empty() {
        return;
    }

    println!("Sending usage report");
    // Here you would gather and send the usage report
    match reqwest::Client::new()
        .post(endpoint)
        .json(to_send.as_slice())
        .send()
        .await
    {
        Ok(response) => {
            tracing::trace!("Usage report sent successfully: {:?}", response);
        }
        Err(e) => {
            tracing::trace!("Failed to send usage report: {:?}", e);
        }
    }
}

struct UsageReportingTask {
    endpoint: String,
    reports: Arc<Mutex<Vec<UsageReport>>>,
    interval: Duration,
}

#[async_trait]
impl BackgroundTask for UsageReportingTask {
    fn id(&self) -> &str {
        "usage_reporting_plugin"
    }
    async fn run(&self, token: CancellationToken) {
        loop {
            ntex::time::sleep(self.interval).await;

            if token.is_cancelled() {
                tracing::trace!("Background task cancelled");

                break;
            }

            flush_reports(&self.endpoint, &self.reports).await;
        }
    }
}
