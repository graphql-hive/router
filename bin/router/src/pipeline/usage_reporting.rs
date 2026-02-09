use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use graphql_tools::parser::schema::Document;
use hive_console_sdk::agent::usage_agent::{AgentError, UsageAgentExt};
use hive_console_sdk::agent::usage_agent::{ExecutionReport, UsageAgent};
use hive_router_config::{
    telemetry::hive::{is_slug_target_ref, is_uuid_target_ref, HiveTelemetryConfig},
    usage_reporting::UsageReportingConfig,
};
use hive_router_internal::{
    background_tasks::{BackgroundTask, BackgroundTasksManager},
    telemetry::resolve_value_or_expression,
};
use hive_router_plan_executor::execution::client_request_details::ClientRequestDetails;
use rand::RngExt;
use tokio_util::sync::CancellationToken;

use crate::consts::ROUTER_VERSION;

#[derive(Debug, thiserror::Error)]
pub enum UsageReportingError {
    #[error("Usage Reporting - Access token is missing. Please provide it via 'HIVE_ACCESS_TOKEN' environment variable or under 'telemetry.hive.token' in the configuration.")]
    MissingAccessToken,
    #[error("Failed to initialize usage agent: {0}")]
    AgentCreationError(#[from] AgentError),
    #[error("Usage Reporting - Configuration error: {0}")]
    ConfigurationError(String),
}

pub fn init_hive_usage_agent(
    bg_tasks_manager: &mut BackgroundTasksManager,
    hive_config: &HiveTelemetryConfig,
) -> Result<UsageAgent, UsageReportingError> {
    let usage_config = &hive_config.usage_reporting;
    let user_agent = format!("hive-router/{}", ROUTER_VERSION);
    let access_token = match &hive_config.token {
        Some(t) => resolve_value_or_expression(t, "Hive Telemetry token")
            .map_err(|e| UsageReportingError::ConfigurationError(e.to_string()))?,
        None => return Err(UsageReportingError::MissingAccessToken),
    };

    let target = match &hive_config.target {
        Some(t) => Some(
            resolve_value_or_expression(t, "Hive Telemetry target")
                .map_err(|e| UsageReportingError::ConfigurationError(e.to_string()))?,
        ),
        None => None,
    };

    if let Some(target) = &target {
        if !is_uuid_target_ref(target) && !is_slug_target_ref(target) {
            return Err(UsageReportingError::ConfigurationError(format!(
                "Invalid Hive Telemetry target format: '{}'. It must be either in slug format '$organizationSlug/$projectSlug/$targetSlug' or UUID format 'a0f4c605-6541-4350-8cfe-b31f21a4bf80'",
                target
            )));
        }
    }

    let mut agent_builder = UsageAgent::builder()
        .user_agent(user_agent)
        .endpoint(usage_config.endpoint.clone())
        .token(access_token)
        .buffer_size(usage_config.buffer_size)
        .connect_timeout(usage_config.connect_timeout)
        .request_timeout(usage_config.request_timeout)
        .accept_invalid_certs(usage_config.accept_invalid_certs)
        .flush_interval(usage_config.flush_interval);

    if let Some(target_id) = target {
        agent_builder = agent_builder.target_id(target_id);
    }

    let agent = agent_builder.build()?;

    bg_tasks_manager.register_task(UsageAgentTask(agent.clone()));
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
    client_request_details: &ClientRequestDetails<'a>,
    hive_usage_agent: &UsageAgent,
    usage_config: &UsageReportingConfig,
    error_count: usize,
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
        operation_body: client_request_details.operation.query.to_owned(),
        operation_name: client_request_details
            .operation
            .name
            .map(|op_name| op_name.to_owned()),
        persisted_document_hash: None,
    };

    if let Err(err) = hive_usage_agent.add_report(execution_report).await {
        tracing::error!("Failed to send usage report: {}", err);
    }
}

struct UsageAgentTask(UsageAgent);

#[async_trait]
impl BackgroundTask for UsageAgentTask {
    fn id(&self) -> &str {
        "hive_console_usage_report_task"
    }

    async fn run(&self, token: CancellationToken) {
        self.0.start_flush_interval(&token).await
    }
}
