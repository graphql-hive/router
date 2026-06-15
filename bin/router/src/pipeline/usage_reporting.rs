use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use graphql_tools::parser::schema::Document;
use hive_console_sdk::agent::usage_agent::{
    AgentError, ExecutionReport, OperationType, RequestDetails, SamplingKey, UsageAgent,
    UsageAgentExt,
};
use hive_router_config::{
    headers::OneOrMany,
    telemetry::hive::{is_slug_target_ref, is_uuid_target_ref, HiveTelemetryConfig},
    usage_reporting::{UsageReportingExclude, UsageReportingSamplingKeyKind},
};
use hive_router_internal::background_tasks::{BackgroundTask, BackgroundTasksManager};
use hive_router_internal::telemetry::utils::resolve_value_or_expression;
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
        .sample_rate(usage_config.sampling.rate.as_f64())
        .buffer_size(usage_config.buffer_size)
        .connect_timeout(usage_config.connect_timeout)
        .request_timeout(usage_config.request_timeout)
        .accept_invalid_certs(usage_config.accept_invalid_certs)
        .flush_interval(usage_config.flush_interval);

    if let Some(target_id) = target {
        agent_builder = agent_builder.target_id(target_id);
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

fn map_sampling_key_to_sdk(kind: &UsageReportingSamplingKeyKind) -> SamplingKey {
    match kind {
        UsageReportingSamplingKeyKind::OperationName => SamplingKey::OperationName,
        UsageReportingSamplingKeyKind::OperationType => SamplingKey::OperationType,
        UsageReportingSamplingKeyKind::OperationBody => SamplingKey::OperationBody,
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
