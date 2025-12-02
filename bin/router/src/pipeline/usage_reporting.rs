use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use graphql_parser::schema::Document;
use hive_console_sdk::agent::{AgentError, ExecutionReport, UsageAgent, UsageAgentExt};
use hive_router_config::usage_reporting::UsageReportingConfig;
use hive_router_plan_executor::execution::{
    client_request_details::ClientRequestDetails, plan::PlanExecutionOutput,
};
use ntex::web::HttpRequest;
use rand::Rng;
use tokio_util::sync::CancellationToken;

use crate::{
    background_tasks::{BackgroundTask, BackgroundTasksManager},
    consts::ROUTER_VERSION,
};

#[derive(Debug, thiserror::Error)]
pub enum UsageReportingError {
    #[error("Usage Reporting - Access token is missing. Please provide it via 'HIVE_ACCESS_TOKEN' environment variable or under 'usage_reporting.access_token' in the configuration.")]
    MissingAccessToken,
    #[error("Usage Reporting - Failed to initialize usage agent: {0}")]
    AgentCreationError(#[from] AgentError),
}

pub fn init_hive_user_agent(
    bg_tasks_manager: &mut BackgroundTasksManager,
    usage_config: &UsageReportingConfig,
) -> Result<Arc<UsageAgent>, UsageReportingError> {
    let user_agent = format!("hive-router/{}", ROUTER_VERSION);
    let access_token = usage_config
        .access_token
        .as_deref()
        .ok_or(UsageReportingError::MissingAccessToken)?;

    let hive_user_agent = UsageAgent::try_new(
        access_token,
        usage_config.endpoint.clone(),
        usage_config.target_id.clone(),
        usage_config.buffer_size,
        usage_config.connect_timeout,
        usage_config.request_timeout,
        usage_config.accept_invalid_certs,
        usage_config.flush_interval,
        user_agent,
    )?;
    bg_tasks_manager.register_task(hive_user_agent.clone());
    Ok(hive_user_agent)
}

#[inline]
pub fn collect_usage_report(
    schema: Arc<Document<'static, String>>,
    duration: Duration,
    req: &HttpRequest,
    client_request_details: &ClientRequestDetails,
    hive_usage_agent: &Arc<UsageAgent>,
    usage_config: &UsageReportingConfig,
    execution_result: &PlanExecutionOutput,
) {
    let sample_rate = usage_config.sample_rate.as_f64();
    if sample_rate < 1.0 && !rand::rng().random_bool(sample_rate) {
        return;
    }
    if client_request_details
        .operation
        .name
        .is_some_and(|op_name| usage_config.exclude.iter().any(|s| s == op_name))
    {
        return;
    }
    let client_name = get_header_value(req, &usage_config.client_name_header);
    let client_version = get_header_value(req, &usage_config.client_version_header);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let execution_report = ExecutionReport {
        schema,
        client_name: client_name.map(|s| s.to_owned()),
        client_version: client_version.map(|s| s.to_owned()),
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

    if let Err(err) = hive_usage_agent.add_report(execution_report) {
        tracing::error!("Failed to send usage report: {}", err);
    }
}

#[inline]
fn get_header_value<'req>(req: &'req HttpRequest, header_name: &str) -> Option<&'req str> {
    req.headers().get(header_name).and_then(|v| v.to_str().ok())
}

#[async_trait]
impl BackgroundTask for UsageAgent {
    fn id(&self) -> &str {
        "hive_console_usage_report_task"
    }

    async fn run(&self, token: CancellationToken) {
        self.start_flush_interval(Some(token)).await
    }
}
