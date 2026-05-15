use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use graphql_tools::parser::schema::Document;
use hive_console_sdk::agent::usage_agent::{
    AgentError, ExecutionReport, OperationType, RequestDetails, UsageAgent, UsageAgentExt,
};
use hive_console_sdk::primitives::target_id::TargetId;
use hive_router_config::primitives::value_or_expression::ValueOrExpression;
use hive_router_config::telemetry::hive::HiveTelemetryConfig;
use hive_router_internal::background_tasks::{BackgroundTask, BackgroundTasksManager};
use hive_router_internal::telemetry::utils::{
    evaluate_expression_as_string, resolve_value_or_expression,
};
use hive_router_query_planner::state::supergraph_state::OperationKind;
use tokio_util::sync::CancellationToken;

use crate::consts::ROUTER_VERSION;

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

    let target: Option<TargetId> = match &hive_config.target {
        Some(ValueOrExpression::Value(t)) => Some(t.clone()),
        Some(ValueOrExpression::Expression { expression }) => {
            let resolved = evaluate_expression_as_string(expression, "Hive Telemetry target")
                .map_err(|e| UsageReportingError::ConfigurationError(e.to_string()))?;
            Some(
                TargetId::parse(resolved)
                    .map_err(|e| UsageReportingError::ConfigurationError(e.to_string()))?,
            )
        }
        None => None,
    };

    let mut agent_builder = UsageAgent::builder()
        .user_agent(user_agent)
        .token(access_token)
        .from_config(usage_config)?;

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
    operation_name: Option<&'a str>,
    operation_kind: Option<&'a OperationKind>,
    operation_body: &'a str,
    hive_usage_agent: &UsageAgent,
    error_count: usize,
    request_details: Option<RequestDetails>,
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
        persisted_document_hash: None,
    };

    if let Err(err) = hive_usage_agent
        .add_report_with_request(execution_report, request_details)
        .await
    {
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
