use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use graphql_parser::schema::Document;
use hive_console_sdk::agent::{ExecutionReport, UsageAgent};
use hive_router_config::{usage_reporting::UsageReportingConfig, HiveRouterConfig};
use hive_router_plan_executor::execution::{
    client_request_details::ClientRequestDetails, plan::PlanExecutionOutput,
};
use ntex::web::HttpRequest;
use rand::Rng;
use tokio_util::sync::CancellationToken;

use crate::background_tasks::BackgroundTask;

pub fn from_config(router_config: &HiveRouterConfig) -> Option<UsageAgent> {
    router_config.usage_reporting.as_ref().map(|usage_config| {
        let flush_interval = Duration::from_secs(usage_config.flush_interval);
        hive_console_sdk::agent::UsageAgent::new(
            usage_config.token.clone(),
            usage_config.endpoint.clone(),
            usage_config.target_id.clone(),
            usage_config.buffer_size,
            usage_config.connect_timeout,
            usage_config.request_timeout,
            usage_config.accept_invalid_certs,
            flush_interval,
            "hive-router".to_string(),
        )
    })
}

pub fn send_usage_report(
    schema: Arc<Document<'static, String>>,
    start: Instant,
    req: &HttpRequest,
    client_request_details: &ClientRequestDetails,
    usage_agent: &UsageAgent,
    usage_config: &UsageReportingConfig,
    execution_result: &PlanExecutionOutput,
) {
    let mut rng = rand::rng();
    let sampled = rng.random::<f64>() < usage_config.sample_rate;
    if !sampled {
        return;
    }
    if client_request_details
        .operation
        .name
        .is_some_and(|op_name| usage_config.exclude.contains(&op_name.to_string()))
    {
        return;
    }
    let client_name = get_header_value(req, &usage_config.client_name_header);
    let client_version = get_header_value(req, &usage_config.client_version_header);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        * 1000;
    let duration = start.elapsed();
    let execution_report = ExecutionReport {
        schema,
        client_name,
        client_version,
        timestamp,
        duration,
        ok: execution_result.error_count == 0,
        errors: execution_result.error_count,
        operation_body: client_request_details.operation.query.to_owned(),
        operation_name: client_request_details
            .operation
            .name
            .map(|op_name| op_name.to_owned()),
        persisted_document_hash: None,
    };
    usage_agent
        .add_report(execution_report)
        .unwrap_or_else(|err| tracing::error!("Failed to send usage report: {}", err));
}

fn get_header_value(req: &HttpRequest, header_name: &str) -> Option<String> {
    req.headers()
        .get(header_name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

#[async_trait]
impl BackgroundTask for UsageAgent {
    fn id(&self) -> &str {
        "usage_report_flush_interval"
    }

    async fn run(&self, token: CancellationToken) {
        self.start_flush_interval(Some(token)).await
    }
}
