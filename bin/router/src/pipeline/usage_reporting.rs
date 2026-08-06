use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::consts::ROUTER_VERSION;
use async_trait::async_trait;
use futures::{stream::FuturesUnordered, StreamExt};
use graphql_tools::parser::schema::Document;
use hive_console_sdk::agent::usage_agent::{
    AgentError, ExecutionReport, OperationType, RequestDetails, SamplingKey, UsageAgent,
    UsageAgentExt,
};
use hive_router_config::{
    headers::OneOrMany,
    telemetry::hive::HiveTelemetryConfig,
    usage_reporting::{UsageReportingExclude, UsageReportingSamplingKeyKind},
};
use hive_router_internal::telemetry::utils::resolve_value_or_expression;
use hive_router_internal::{
    background_tasks::{BackgroundTask, CancellationToken},
    telemetry::logging::targets,
};
use hive_router_query_planner::state::supergraph_state::OperationKind;
use tokio::sync::{mpsc, Mutex};
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub enum UsageReportingError {
    #[error(
        "Usage Reporting - Access token is missing. Please provide it via 'HIVE_ACCESS_TOKEN' environment variable or under 'telemetry.hive.token' in the configuration."
    )]
    MissingAccessToken,
    #[error("Failed to initialize usage agent: {0}")]
    AgentCreationError(#[from] AgentError),
    #[error("Usage Reporting - Configuration error: {0}")]
    ConfigurationError(String),
}

pub fn init_hive_usage_agent(
    background_tasks: &HiveUsageReportingBackgroundTaskController,
    supergraph_lifetime: CancellationToken,
    hive_config: &HiveTelemetryConfig,
    target: Option<&str>,
) -> Result<UsageAgent, UsageReportingError> {
    let usage_config = &hive_config.usage_reporting;
    let user_agent = format!("hive-router/{}", ROUTER_VERSION);
    let access_token = match &hive_config.token {
        Some(t) => resolve_value_or_expression(t, "Hive Telemetry token")
            .map_err(|e| UsageReportingError::ConfigurationError(e.to_string()))?,
        None => return Err(UsageReportingError::MissingAccessToken),
    };

    let mut agent_builder = UsageAgent::builder()
        .user_agent(user_agent)
        .endpoint(usage_config.endpoint.clone())
        .token(access_token)
        .sample_rate(usage_config.sampling.rate.as_f64())
        .buffer_size(usage_config.buffer_size)
        .connect_timeout(usage_config.connect_timeout)
        .request_timeout(usage_config.request_timeout)
        .accept_invalid_certs(usage_config.accept_invalid_certs)
        .flush_interval(usage_config.flush_interval);

    if let Some(target_id) = target {
        agent_builder = agent_builder.target_id(target_id.to_string());
    }

    if let Some(UsageReportingExclude::Expression { expression }) = &usage_config.exclude {
        agent_builder = agent_builder.exclude_expression(expression.clone());
    }

    if let Some(UsageReportingExclude::OperationNames(operation_names)) = &usage_config.exclude {
        agent_builder = agent_builder.exclude_operation_names(operation_names.clone());
    }

    if let Some(at_least_once) = &usage_config.sampling.at_least_once {
        agent_builder = agent_builder.at_least_once_sampling(
            match &at_least_once.key {
                OneOrMany::One(kind) => vec![map_sampling_key_to_sdk(kind)],
                OneOrMany::Many(kinds) => kinds.iter().map(map_sampling_key_to_sdk).collect(),
            },
            at_least_once.max_distinct_keys,
        );
    }

    let agent = agent_builder.build()?;

    background_tasks.add_worker(agent.clone(), supergraph_lifetime);
    Ok(agent)
}

// TODO: simplfy args
#[allow(clippy::too_many_arguments)]
#[inline]
pub async fn collect_usage_report<'a>(
    schema: Arc<Document<'static, String>>,
    duration: Duration,
    client_name: Option<&str>,
    client_version: Option<&str>,
    operation_name: Option<&'a str>,
    operation_kind: Option<&'a OperationKind>,
    operation_body: &'a str,
    hive_usage_agent: &UsageAgent,
    error_count: usize,
    request_details: Option<RequestDetails>,
    persisted_document_hash: Option<&str>,
) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let execution_report = ExecutionReport {
        schema,
        client_name: client_name.map(|name| name.to_string()),
        client_version: client_version.map(|version| version.to_string()),
        timestamp,
        duration,
        ok: error_count == 0,
        errors: error_count,
        operation_body: operation_body.to_owned(),
        operation_type: operation_kind.map(|k| match k {
            OperationKind::Query => OperationType::Query,
            OperationKind::Mutation => OperationType::Mutation,
            OperationKind::Subscription => OperationType::Subscription,
        }),
        operation_name: operation_name.map(|s| s.to_owned()),
        persisted_document_hash: persisted_document_hash.map(|hash| hash.to_owned()),
    };

    if let Err(err) = hive_usage_agent
        .add_report_with_request(execution_report, request_details)
        .await
    {
        error!(target: targets::HIVE_USAGE_REPORTING, error = ?err, "failed to send usage report to hive");
    }
}

fn map_sampling_key_to_sdk(kind: &UsageReportingSamplingKeyKind) -> SamplingKey {
    match kind {
        UsageReportingSamplingKeyKind::OperationName => SamplingKey::OperationName,
        UsageReportingSamplingKeyKind::OperationType => SamplingKey::OperationType,
        UsageReportingSamplingKeyKind::OperationBody => SamplingKey::OperationBody,
    }
}

struct HiveUsageReportingWorker {
    agent: UsageAgent,
    supergraph_lifetime: CancellationToken,
}

fn hive_usage_reporting_worker(
    worker: HiveUsageReportingWorker,
    router_shutdown: CancellationToken,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        let interval_token = CancellationToken::new();
        let interval = worker.agent.start_flush_interval(&interval_token);
        tokio::pin!(interval);
        let cancelled = tokio::select! {
            _ = router_shutdown.cancelled() => true,
            _ = worker.supergraph_lifetime.cancelled() => true,
            _ = &mut interval => false,
        };
        if cancelled {
            interval_token.cancel();
            interval.await;
        }
        if let Err(err) = worker.agent.flush().await {
            error!(target: targets::HIVE_USAGE_REPORTING, error = ?err, "failed to flush usage reports while stopping supergraph runtime");
        }
    })
}

/// Adds Hive usage-reporting workers that are scoped to one selected supergraph runtime.
///
/// The supplied lifetime is the cancellation token owned by that runtime. It is cancelled only
/// after the final runtime reference is dropped, so reports from retained requests, WebSockets, and
/// subscriptions are not lost after cache eviction or supergraph retirement. Cancellation removes
/// the worker only after its buffered reports have been flushed.
#[derive(Clone)]
pub struct HiveUsageReportingBackgroundTaskController {
    sender: mpsc::UnboundedSender<HiveUsageReportingWorker>,
}

impl HiveUsageReportingBackgroundTaskController {
    fn add_worker(&self, agent: UsageAgent, supergraph_lifetime: CancellationToken) {
        self.sender
            .send(HiveUsageReportingWorker {
                agent,
                supergraph_lifetime,
            })
            .ok();
    }
}

/// Runs Hive usage-reporting agents registered for selected supergraph runtimes.
///
/// Each worker periodically flushes one runtime's agent. When either the router or that runtime
/// shuts down, the periodic loop is stopped and one final flush is awaited before the worker is
/// removed. Explicitly awaiting this flush avoids relying on asynchronous drop timing.
pub struct HiveUsageReportingBackgroundTasks {
    registrations: Mutex<mpsc::UnboundedReceiver<HiveUsageReportingWorker>>,
}

impl HiveUsageReportingBackgroundTasks {
    pub fn new() -> (HiveUsageReportingBackgroundTaskController, Self) {
        let (sender, registrations) = mpsc::unbounded_channel();
        (
            HiveUsageReportingBackgroundTaskController { sender },
            Self {
                registrations: Mutex::new(registrations),
            },
        )
    }
}

#[async_trait]
impl BackgroundTask for HiveUsageReportingBackgroundTasks {
    fn id(&self) -> &str {
        "hive-usage-reporting-background-tasks"
    }

    async fn run(&self, router_shutdown: CancellationToken) {
        let mut registrations = self.registrations.lock().await;
        let mut workers: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>> =
            FuturesUnordered::new();

        loop {
            tokio::select! {
                _ = router_shutdown.cancelled() => {
                    registrations.close();
                    while let Some(worker) = registrations.recv().await {
                        workers.push(hive_usage_reporting_worker(
                            worker,
                            router_shutdown.clone(),
                        ));
                    }
                    while workers.next().await.is_some() {}
                    return;
                }
                registration = registrations.recv() => {
                    let Some(worker) = registration else {
                        while workers.next().await.is_some() {}
                        return;
                    };
                    workers.push(hive_usage_reporting_worker(
                        worker,
                        router_shutdown.clone(),
                    ));
                }
                Some(()) = workers.next(), if !workers.is_empty() => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphql_tools::parser::schema::parse_schema;

    // usage agent async drop uses block_in_place, which requires tokio's multi-threaded runtime
    #[tokio::test(flavor = "multi_thread")]
    async fn lifetime_cancellation_flushes_before_the_worker_stops() {
        crate::init_rustls_crypto_provider();
        let mut server = mockito::Server::new_async().await;
        let usage_request = server
            .mock("POST", "/usage")
            .expect(1)
            .with_status(200)
            .create_async()
            .await;
        let agent = UsageAgent::builder()
            .token("token".into())
            .endpoint(format!("{}/usage", server.url()))
            .flush_interval(Duration::from_secs(3600))
            .build()
            .unwrap();
        agent
            .add_report_with_request(
                ExecutionReport {
                    schema: Arc::new(
                        parse_schema::<String>("type Query { hello: String }")
                            .unwrap()
                            .to_owned(),
                    ),
                    client_name: None,
                    client_version: None,
                    timestamp: 0,
                    duration: Duration::ZERO,
                    ok: true,
                    errors: 0,
                    operation_body: "query { hello }".to_string(),
                    operation_type: Some(OperationType::Query),
                    operation_name: None,
                    persisted_document_hash: None,
                },
                None,
            )
            .await
            .unwrap();

        let (controller, background_tasks) = HiveUsageReportingBackgroundTasks::new();
        let router_shutdown = CancellationToken::new();
        let supergraph_lifetime = CancellationToken::new();
        controller.add_worker(agent, supergraph_lifetime.clone());
        let task_shutdown = router_shutdown.clone();
        let handle = tokio::spawn(async move {
            background_tasks.run(task_shutdown).await;
        });

        supergraph_lifetime.cancel();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !usage_request.matched_async().await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("usage worker should flush promptly after lifetime cancellation");

        router_shutdown.cancel();
        handle.await.unwrap();
        usage_request.assert_async().await;
    }
}

#[inline]
pub fn request_details_from_ntex_request(req: &ntex::web::HttpRequest) -> RequestDetails {
    let mut headers = Vec::with_capacity(req.headers().len());
    for (name, value) in req.headers().iter() {
        if let Ok(val_str) = value.to_str() {
            headers.push((name.to_string(), val_str.to_string()));
        }
    }

    RequestDetails {
        method: req.method().clone(),
        url: req.uri().clone(),
        headers,
    }
}
