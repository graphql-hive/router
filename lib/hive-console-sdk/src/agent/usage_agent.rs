use graphql_tools::parser::schema::Document;
use rand::prelude::*;
use recloser::AsyncRecloser;
use reqwest_middleware::ClientWithMiddleware;
use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap},
    hash::{Hash, Hasher},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::{agent::utils::ReportVariables, expressions::lib::FromVrlValue};
use crate::{
    agent::{buffer::AddStatus, utils::OperationProcessor},
    expressions::ExecutableProgram,
    helpers::SharedFifoSet,
};
use crate::{
    agent::{buffer::Buffer, builder::UsageAgentBuilder},
    expressions::values::boolean::BooleanConversionError,
};
use vrl::{compiler::Program as VrlProgram, core::Value as VrlValue, value::KeyString};
use xxhash_rust::xxh3::Xxh3;

const USAGE_REPORTING_TARGET: &str = "console_sdk::usage_reporting";

#[derive(Debug, Clone, Default)]
pub enum OperationType {
    #[default]
    Query,
    Mutation,
    Subscription,
}

#[derive(Debug, Default, Clone)]
pub struct ExecutionReport {
    pub schema: Arc<Document<'static, String>>,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
    pub timestamp: u64,
    pub duration: Duration,
    pub ok: bool,
    pub errors: usize,
    pub operation_body: String,
    pub operation_name: Option<String>,
    pub operation_type: Option<OperationType>,
    pub persisted_document_hash: Option<String>,
    /// The raw variables of the execution, as received before coercion.
    /// Only used when `process_variables` is enabled.
    pub variables: Option<ReportVariables>,
}

typify::import_types!(schema = "./usage-report-v2.schema.json");

pub struct UsageAgentInner {
    pub(crate) endpoint: String,
    pub(crate) buffer: Buffer<ExecutionReport>,
    pub(crate) processor: OperationProcessor,
    pub(crate) client: ClientWithMiddleware,
    pub(crate) flush_interval: Duration,
    pub(crate) circuit_breaker: AsyncRecloser,
    pub(crate) exclude: Option<Exclude>,
    pub(crate) sample_rate: f64,
    pub(crate) at_least_once: Option<AtLeastOnceSampling>,
}

#[derive(Debug, Clone)]
pub enum Exclude {
    OperationNames(Vec<String>),
    Expression(Box<VrlProgram>),
}

#[derive(Debug, Clone, Copy)]
pub enum SamplingKey {
    OperationName,
    OperationType,
    OperationBody,
}

pub struct AtLeastOnceSampling {
    pub(crate) key: Vec<SamplingKey>,
    pub(crate) seen_hashes: SharedFifoSet,
}

pub fn non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|str| !str.is_empty())
}

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("unable to acquire lock: {0}")]
    Lock(String),
    #[error("unable to send report: unauthorized")]
    Unauthorized,
    #[error("unable to send report: no access")]
    Forbidden,
    #[error("unable to send report: rate limited")]
    RateLimited,
    #[error("missing token")]
    MissingToken,
    #[error("your access token requires providing a 'target_id' option.")]
    MissingTargetId,
    #[error("using 'target_id' with legacy tokens is not supported")]
    TargetIdWithLegacyToken,
    #[error("invalid token provided")]
    InvalidToken,
    #[error(
        "invalid target id provided: {0}, it should be either a slug like \"$organizationSlug/$projectSlug/$targetSlug\" or an UUID"
    )]
    InvalidTargetId(String),
    #[error("unable to instantiate the http client for reports sending: {0}")]
    HTTPClientCreationError(reqwest::Error),
    #[error("unable to create circuit breaker: {0}")]
    CircuitBreakerCreationError(#[from] crate::circuit_breaker::CircuitBreakerError),
    #[error("rejected by the circuit breaker")]
    CircuitBreakerRejected,
    #[error("unable to send report: {0}")]
    Unknown(String),
    #[error("failed to compile exclude expression: {0}")]
    ExcludeExpressionCompileError(#[from] crate::expressions::ExpressionCompileError),
    #[error("failed to execute exclude expression: {0}")]
    ExcludeExpressionExecutionError(#[from] crate::expressions::ExpressionExecutionError),
    #[error("failed to convert exclude expression result to boolean: {0}")]
    ExcludeExpressionResultConversionError(#[from] BooleanConversionError),
}

pub struct UsageAgentHandle(Option<UsageAgentInner>);

impl UsageAgentHandle {
    pub(crate) fn new(inner: UsageAgentInner) -> Self {
        Self(Some(inner))
    }

    pub fn should_process_variables(&self) -> bool {
        self.inner().processor.process_variables_enabled
    }

    fn inner(&self) -> &UsageAgentInner {
        self.0.as_ref().expect("UsageAgentHandle used after drop")
    }
}

impl Drop for UsageAgentHandle {
    fn drop(&mut self) {
        if let Some(inner) = self.0.take() {
            tokio::spawn(async move {
                if let Err(e) = inner.flush().await {
                    tracing::error!(target: USAGE_REPORTING_TARGET, error = ?e, "Failed to flush usage reports during drop");
                }
            });
        }
    }
}

pub type UsageAgent = Arc<UsageAgentHandle>;

#[async_trait::async_trait]
pub trait UsageAgentExt {
    fn builder() -> UsageAgentBuilder {
        UsageAgentBuilder::default()
    }
    async fn flush(&self) -> Result<(), AgentError>;
    async fn start_flush_interval(&self, token: &CancellationToken);
    /// Deprecated: use [`add_report_with_request`] instead.
    /// This method will be removed in a future version major.
    #[deprecated(note = "use `add_report_with_request` instead")]
    async fn add_report(&self, execution_report: ExecutionReport) -> Result<(), AgentError>;

    async fn add_report_with_request(
        &self,
        execution_report: ExecutionReport,
        request: Option<RequestDetails>,
    ) -> Result<(), AgentError>;
}

impl UsageAgentInner {
    fn should_exclude(
        &self,
        execution_report: &ExecutionReport,
        request: Option<&RequestDetails>,
    ) -> Result<bool, AgentError> {
        self.exclude.as_ref().map_or(Ok(false), |exclude| {
            exclude.should_exclude(execution_report, request)
        })
    }

    fn should_sample(&self, execution_report: &ExecutionReport) -> bool {
        if let Some(at_least_once) = &self.at_least_once {
            let key_hash = at_least_once.resolve_key_hash(execution_report);
            // Every first distinct report should be sampled
            if at_least_once.mark_seen(key_hash) {
                return true;
            }
        }

        let sample_rate = self.sample_rate;
        if sample_rate >= 1.0 {
            return true;
        }
        if sample_rate <= 0.0 {
            return false;
        }

        rand::rng().random_bool(sample_rate)
    }

    fn produce_report(&self, reports: Vec<ExecutionReport>) -> Result<Report, AgentError> {
        let mut report = Report {
            size: 0,
            map: HashMap::new(),
            operations: Vec::new(),
            subscription_operations: Vec::new(),
        };

        // iterate over reports and check if they are valid
        for op in reports {
            let operation =
                self.processor
                    .process(&op.operation_body, &op.schema, op.variables.as_ref());

            match operation {
                Err(e) => {
                    tracing::warn!(
                        target: USAGE_REPORTING_TARGET,
                        error = ?e,
                        operation_name = op.operation_name
                            .clone()
                            .or_else(|| Some("anonymous".to_string()))
                            .unwrap(),
                        phase = "PROCESSING",
                        "Dropping operation",
                    );

                    continue;
                }
                Ok(operation) => match operation {
                    Some(operation) => {
                        let hash = operation.hash;
                        let client_name = non_empty_string(op.client_name);
                        let client_version = non_empty_string(op.client_version);

                        let metadata: Option<Metadata> =
                            if client_name.is_some() || client_version.is_some() {
                                Some(Metadata {
                                    client: Some(Client {
                                        name: client_name.unwrap_or_default(),
                                        version: client_version.unwrap_or_default(),
                                    }),
                                })
                            } else {
                                None
                            };
                        report.operations.push(RequestOperation {
                            operation_map_key: hash.clone(),
                            timestamp: op.timestamp,
                            execution: Execution {
                                ok: op.ok,
                                /*
                                    The conversion from u128 (from op.duration.as_nanos()) to u64 using try_into().unwrap() can panic if the duration is longer than u64::MAX nanoseconds (over 584 years).
                                    While highly unlikely, it's safer to handle this potential overflow gracefully in library code to prevent panics.
                                    A safe alternative is to convert the Result to an Option and provide a fallback value on failure,
                                    effectively saturating at u64::MAX.
                                */
                                duration: op
                                    .duration
                                    .as_nanos()
                                    .try_into()
                                    .ok()
                                    .unwrap_or(u64::MAX),
                                errors_total: op.errors.try_into().unwrap(),
                            },
                            persisted_document_hash: op
                                .persisted_document_hash
                                .map(PersistedDocumentHash),
                            metadata,
                        });
                        if let Entry::Vacant(e) = report.map.entry(ReportMapKey(hash)) {
                            e.insert(OperationMapRecord {
                                operation: operation.operation,
                                operation_name: non_empty_string(op.operation_name),
                                fields: operation.coordinates,
                            });
                        }
                        report.size += 1;
                    }
                    None => {
                        tracing::debug!(
                            target: USAGE_REPORTING_TARGET,
                            phase = "PROCESSING",
                            "Dropping operation, probably introspection query"
                        );
                    }
                },
            }
        }

        Ok(report)
    }

    async fn send_report(&self, report: Report) -> Result<(), AgentError> {
        if report.size == 0 {
            return Ok(());
        }
        // Based on https://the-guild.dev/graphql/hive/docs/specs/usage-reports#data-structure
        let resp_fut = self.client.post(&self.endpoint).json(&report).send();

        let resp = self
            .circuit_breaker
            .call(resp_fut)
            .await
            .map_err(|e| match e {
                recloser::Error::Inner(e) => AgentError::Unknown(e.to_string()),
                recloser::Error::Rejected => AgentError::CircuitBreakerRejected,
            })?;

        match resp.status() {
            reqwest::StatusCode::OK => Ok(()),
            reqwest::StatusCode::UNAUTHORIZED => Err(AgentError::Unauthorized),
            reqwest::StatusCode::FORBIDDEN => Err(AgentError::Forbidden),
            reqwest::StatusCode::TOO_MANY_REQUESTS => Err(AgentError::RateLimited),
            _ => Err(AgentError::Unknown(format!(
                "({}) {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            ))),
        }
    }

    async fn handle_drained(&self, drained: Vec<ExecutionReport>) -> Result<(), AgentError> {
        if drained.is_empty() {
            return Ok(());
        }
        let report = self.produce_report(drained)?;
        self.send_report(report).await
    }

    async fn flush(&self) -> Result<(), AgentError> {
        let execution_reports = self.buffer.drain().await;

        self.handle_drained(execution_reports).await?;

        Ok(())
    }
}

impl Exclude {
    fn should_exclude(
        &self,
        execution_report: &ExecutionReport,
        request: Option<&RequestDetails>,
    ) -> Result<bool, AgentError> {
        match self {
            Exclude::OperationNames(operation_names) => Ok(execution_report
                .operation_name
                .as_deref()
                .is_some_and(|operation_name| {
                    operation_names.iter().any(|name| name == operation_name)
                })),
            Exclude::Expression(program) => {
                let result = program.execute(get_vrl_value_from_execution_report_and_request(
                    execution_report,
                    request.cloned(),
                ))?;
                bool::from_vrl_value(result).map_err(AgentError::from)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RequestDetails {
    pub method: http::Method,
    pub url: http::Uri,
    pub headers: Vec<(String, String)>,
}

#[async_trait::async_trait]
impl UsageAgentExt for UsageAgent {
    async fn flush(&self) -> Result<(), AgentError> {
        self.inner().flush().await
    }

    async fn start_flush_interval(&self, token: &CancellationToken) {
        loop {
            tokio::select! {
                _ = token.cancelled() => {
                    debug!(target: USAGE_REPORTING_TARGET, "Shutting down.");
                    return;
                }
                _ = tokio::time::sleep(self.inner().flush_interval) => {}
            }

            self.flush()
                .await
                .unwrap_or_else(|e| tracing::error!(target: USAGE_REPORTING_TARGET, error = ?e, "Failed to flush usage reports"));
        }
    }

    async fn add_report_with_request(
        &self,
        execution_report: ExecutionReport,
        request: Option<RequestDetails>,
    ) -> Result<(), AgentError> {
        let inner = self.inner();

        if inner.should_exclude(&execution_report, request.as_ref())? {
            tracing::debug!(
                target: USAGE_REPORTING_TARGET,
                operation_name = execution_report
                    .operation_name
                    .as_deref()
                    .unwrap_or("anonymous"),
                "Excluding report for operation based on exclude expression evaluation",
            );

            return Ok(());
        }

        if !inner.should_sample(&execution_report) {
            tracing::debug!(
                target: USAGE_REPORTING_TARGET,
                operation_name = execution_report
                    .operation_name
                    .as_deref()
                    .unwrap_or("anonymous"),
                "Sampling dropped report for operation",
            );

            return Ok(());
        }

        if let AddStatus::Full { drained } = inner.buffer.add(execution_report).await {
            inner.handle_drained(drained).await?;
        }

        Ok(())
    }
    async fn add_report(&self, execution_report: ExecutionReport) -> Result<(), AgentError> {
        self.add_report_with_request(execution_report, None).await
    }
}

impl<'req, TBody> From<&'req http::Request<TBody>> for RequestDetails {
    fn from(req: &'req http::Request<TBody>) -> Self {
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
}

impl From<RequestDetails> for VrlValue {
    fn from(details: RequestDetails) -> Self {
        let mut merged_headers: BTreeMap<String, String> = BTreeMap::new();
        for (header_name, header_value) in details.headers {
            if let Some(existing_value) = merged_headers.get_mut(&header_name) {
                existing_value.push_str(", ");
                existing_value.push_str(&header_value);
            } else {
                merged_headers.insert(header_name, header_value);
            }
        }

        let headers_value: BTreeMap<KeyString, VrlValue> = merged_headers
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        let headers_value = VrlValue::Object(headers_value);

        // .request.url
        let url_value = VrlValue::Object(BTreeMap::from([
            ("host".into(), details.url.host().unwrap_or_default().into()),
            ("path".into(), details.url.path().into()),
            (
                "port".into(),
                details
                    .url
                    .port_u16()
                    .map(|p| VrlValue::Integer(p.into()))
                    .unwrap_or(VrlValue::Null),
            ),
        ]));

        // .request
        VrlValue::Object(BTreeMap::from([
            ("method".into(), details.method.as_str().into()),
            ("headers".into(), headers_value),
            ("url".into(), url_value),
        ]))
    }
}

impl AtLeastOnceSampling {
    fn resolve_key_hash(&self, report: &ExecutionReport) -> u64 {
        let mut hasher = Xxh3::new();

        for key in &self.key {
            let value = match key {
                SamplingKey::OperationName => {
                    report.operation_name.as_deref().unwrap_or("anonymous")
                }
                SamplingKey::OperationType => match report.operation_type.as_ref() {
                    None | Some(OperationType::Query) => "query",
                    Some(OperationType::Mutation) => "mutation",
                    Some(OperationType::Subscription) => "subscription",
                },
                SamplingKey::OperationBody => report.operation_body.as_str(),
            };
            value.hash(&mut hasher);
            0u8.hash(&mut hasher);
        }

        hasher.finish()
    }

    /// Marks the given key hash as seen, returning true if it was not already seen.
    fn mark_seen(&self, key_hash: u64) -> bool {
        self.seen_hashes.insert(key_hash)
    }
}

pub fn get_vrl_value_from_execution_report_and_request(
    report: &ExecutionReport,
    request: Option<RequestDetails>,
) -> VrlValue {
    let mut map = BTreeMap::from([("default".into(), VrlValue::Boolean(false))]);
    let mut request_map = BTreeMap::new();

    if let Some(request_details) = request {
        if let VrlValue::Object(request_object) = VrlValue::from(request_details) {
            request_map.extend(request_object);
        }
    }

    request_map.insert(
        "operation".into(),
        VrlValue::Object(BTreeMap::from([
            (
                "name".into(),
                report.operation_name.clone().unwrap_or_default().into(),
            ),
            (
                "type".into(),
                report
                    .operation_type
                    .as_ref()
                    .map(|operation_type| match operation_type {
                        OperationType::Query => "query",
                        OperationType::Mutation => "mutation",
                        OperationType::Subscription => "subscription",
                    })
                    .unwrap_or_default()
                    .into(),
            ),
            ("query".into(), report.operation_body.clone().into()),
        ])),
    );

    if let Ok(timestamp_integer) = report.timestamp.try_into() {
        let timestamp_value = VrlValue::Integer(timestamp_integer);
        map.insert("timestamp".into(), timestamp_value);
        request_map.insert("timestamp".into(), VrlValue::Integer(timestamp_integer));
    }

    map.insert("request".into(), VrlValue::Object(request_map));

    VrlValue::Object(map)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use graphql_tools::parser::{parse_query, parse_schema};
    use reqwest::{
        header::{AUTHORIZATION, CONTENT_TYPE, USER_AGENT},
        Method,
    };
    use vrl::core::Value as VrlValue;
    use vrl::value::KeyString;

    use crate::agent::usage_agent::{
        get_vrl_value_from_execution_report_and_request, ExecutionReport, OperationType, Report,
        UsageAgent, UsageAgentExt,
    };

    async fn wait_for_mock(mock: &mockito::Mock) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !mock.matched_async().await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("mock should be matched after usage agent drop flush");
    }

    /// Helper to extract a nested VRL value from an Object using string keys.
    fn vrl_get<'a>(value: &'a VrlValue, keys: &[&str]) -> &'a VrlValue {
        let mut current = value;
        for key in keys {
            match current {
                VrlValue::Object(map) => {
                    let ks: KeyString = (*key).into();
                    current = map
                        .get(&ks)
                        .unwrap_or_else(|| panic!("key '{}' not found", key));
                }
                _ => panic!("expected Object at key '{}'", key),
            }
        }
        current
    }

    const CONTENT_TYPE_VALUE: &'static str = "application/json";
    const GRAPHQL_CLIENT_NAME: &'static str = "Hive Client";
    const GRAPHQL_CLIENT_VERSION: &'static str = "1.0.0";

    #[tokio::test(flavor = "multi_thread")]
    async fn should_send_data_to_hive() -> Result<(), Box<dyn std::error::Error>> {
        let token = "Token";

        let mut server = mockito::Server::new_async().await;

        let server_url = server.url();

        let timestamp = 1625247600;
        let duration = Duration::from_millis(20);
        let user_agent = "hive-router-sdk-test";

        let mock = server
            .mock("POST", "/200")
            .match_header(AUTHORIZATION, format!("Bearer {}", token).as_str())
            .match_header(CONTENT_TYPE, CONTENT_TYPE_VALUE)
            .match_header(USER_AGENT, user_agent)
            .match_header("X-Usage-API-Version", "2")
            .match_request(move |request| {
                let request_body = request.body().expect("Failed to extract body");
                let report: Report = serde_json::from_slice(request_body)
                    .expect("Failed to parse request body as JSON");
                assert_eq!(report.size, 1);
                let record = report.map.values().next().expect("No operation record");
                // operation
                assert!(record.operation.contains("mutation deleteProject"));
                assert_eq!(record.operation_name.as_deref(), Some("deleteProject"));
                // fields
                let expected_fields = vec![
                    "Mutation.deleteProject",
                    "Mutation.deleteProject.selector",
                    "DeleteProjectPayload.selector",
                    "ProjectSelector.organization",
                    "ProjectSelector.project",
                    "DeleteProjectPayload.deletedProject",
                    "Project.id",
                    "Project.cleanId",
                    "Project.name",
                    "Project.type",
                    "ProjectType.FEDERATION",
                    "ProjectType.STITCHING",
                    "ProjectType.SINGLE",
                    "ProjectType.CUSTOM",
                    "ProjectSelectorInput.organization",
                    "ID",
                    "ProjectSelectorInput.project",
                ];
                for field in &expected_fields {
                    assert!(
                        record.fields.contains(&field.to_string()),
                        "Missing field: {}",
                        field
                    );
                }
                assert_eq!(
                    record.fields.len(),
                    expected_fields.len(),
                    "Unexpected number of fields"
                );

                // Operations
                let operations = report.operations;
                assert_eq!(operations.len(), 1); // one operation

                let operation = &operations[0];
                let key = report.map.keys().next().expect("No operation key");
                assert_eq!(operation.operation_map_key, key.0);
                assert_eq!(operation.timestamp, timestamp);
                assert_eq!(operation.execution.duration, duration.as_nanos() as u64);
                assert_eq!(operation.execution.ok, true);
                assert_eq!(operation.execution.errors_total, 0);
                true
            })
            .expect(1)
            .with_status(200)
            .create_async()
            .await;
        let schema: graphql_tools::static_graphql::schema::Document = parse_schema(
            r#"
                type Query {
                    project(selector: ProjectSelectorInput!): Project
                    projectsByType(type: ProjectType!): [Project!]!
                    projects(filter: FilterInput): [Project!]!
                }

                type Mutation {
                    deleteProject(selector: ProjectSelectorInput!): DeleteProjectPayload!
                }

                input ProjectSelectorInput {
                    organization: ID!
                    project: ID!
                }

                input FilterInput {
                    type: ProjectType
                    pagination: PaginationInput
                }

                input PaginationInput {
                    limit: Int
                    offset: Int
                }

                type ProjectSelector {
                    organization: ID!
                    project: ID!
                }

                type DeleteProjectPayload {
                    selector: ProjectSelector!
                    deletedProject: Project!
                }

                type Project {
                    id: ID!
                    cleanId: ID!
                    name: String!
                    type: ProjectType!
                    buildUrl: String
                    validationUrl: String
                }

                enum ProjectType {
                    FEDERATION
                    STITCHING
                    SINGLE
                    CUSTOM
                }
        "#,
        )?;

        let op: graphql_tools::static_graphql::query::Document = parse_query(
            r#"
                mutation deleteProject($selector: ProjectSelectorInput!) {
                    deleteProject(selector: $selector) {
                    selector {
                        organization
                        project
                    }
                    deletedProject {
                        ...ProjectFields
                    }
                    }
                }

                fragment ProjectFields on Project {
                    id
                    cleanId
                    name
                    type
                }
        "#,
        )?;

        // Testing async drop
        {
            let usage_agent = UsageAgent::builder()
                .token(token.into())
                .endpoint(format!("{}/200", server_url))
                .user_agent(user_agent.into())
                .build()?;

            let request = http::Request::builder()
                .method(Method::POST)
                .uri("http://localhost/graphql")
                .body(())
                .unwrap();

            usage_agent
                .add_report_with_request(
                    ExecutionReport {
                        schema: Arc::new(schema),
                        operation_body: op.to_string(),
                        operation_name: Some("deleteProject".to_string()),
                        operation_type: Some(OperationType::Mutation),
                        client_name: Some(GRAPHQL_CLIENT_NAME.to_string()),
                        client_version: Some(GRAPHQL_CLIENT_VERSION.to_string()),
                        timestamp,
                        duration,
                        ok: true,
                        errors: 0,
                        persisted_document_hash: None,
                        variables: None,
                    },
                    Some((&request).into()),
                )
                .await?;
        }

        wait_for_mock(&mock).await;
        mock.assert_async().await;

        Ok(())
    }

    fn make_test_report(
        operation_name: Option<&str>,
        operation_type: OperationType,
        operation_body: &str,
    ) -> ExecutionReport {
        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema("type Query { hello: String }").unwrap();

        ExecutionReport {
            schema: Arc::new(schema),
            operation_body: operation_body.to_string(),
            operation_name: operation_name.map(|s| s.to_string()),
            operation_type: Some(operation_type),
            client_name: Some("test-client".to_string()),
            client_version: Some("1.0.0".to_string()),
            timestamp: 1625247600,
            duration: Duration::from_millis(10),
            ok: true,
            errors: 0,
            persisted_document_hash: None,
            variables: None,
        }
    }

    fn make_simple_report(
        operation_name: Option<&str>,
        operation_type: OperationType,
    ) -> ExecutionReport {
        make_test_report(operation_name, operation_type, "query { hello }")
    }

    fn make_simple_request() -> http::Request<()> {
        http::Request::builder()
            .method(Method::GET)
            .uri("http://localhost/graphql")
            .body(())
            .unwrap()
    }

    #[test]
    fn vrl_value_contains_operation_name() {
        let report = make_simple_report(Some("MyQuery"), OperationType::Query);
        let request = make_simple_request();
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        let name = vrl_get(&value, &["request", "operation", "name"]);
        assert_eq!(name, &VrlValue::from("MyQuery"));
    }

    #[test]
    fn vrl_value_contains_operation_type_query() {
        let report = make_simple_report(Some("Q"), OperationType::Query);
        let request = make_simple_request();
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        let op_type = vrl_get(&value, &["request", "operation", "type"]);
        assert_eq!(op_type, &VrlValue::from("query"));
    }

    #[test]
    fn vrl_value_contains_operation_type_mutation() {
        let report = make_simple_report(Some("M"), OperationType::Mutation);
        let request = make_simple_request();
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        let op_type = vrl_get(&value, &["request", "operation", "type"]);
        assert_eq!(op_type, &VrlValue::from("mutation"));
    }

    #[test]
    fn vrl_value_contains_operation_type_subscription() {
        let report = make_simple_report(Some("S"), OperationType::Subscription);
        let request = make_simple_request();
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        let op_type = vrl_get(&value, &["request", "operation", "type"]);
        assert_eq!(op_type, &VrlValue::from("subscription"));
    }

    #[test]
    fn vrl_value_contains_operation_body() {
        let report = make_simple_report(Some("Q"), OperationType::Query);
        let request = make_simple_request();
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        let query = vrl_get(&value, &["request", "operation", "query"]);
        assert_eq!(query, &VrlValue::from("query { hello }"));
    }

    #[test]
    fn vrl_value_contains_request_method() {
        let report = make_test_report(Some("Q"), OperationType::Query, "query { hello }");
        let request = http::Request::builder()
            .method(Method::GET)
            .uri("http://localhost/graphql")
            .body(())
            .unwrap();
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        let method = vrl_get(&value, &["request", "method"]);
        assert_eq!(method, &VrlValue::from("GET"));
    }

    #[test]
    fn vrl_value_contains_url_details() {
        let report = make_test_report(Some("Q"), OperationType::Query, "query { hello }");
        let request = http::Request::builder()
            .method(Method::POST)
            .uri("http://api.example.com:8080/v1/graphql")
            .body(())
            .unwrap();
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        assert_eq!(
            vrl_get(&value, &["request", "url", "host"]),
            &VrlValue::from("api.example.com")
        );
        assert_eq!(
            vrl_get(&value, &["request", "url", "port"]),
            &VrlValue::Integer(8080)
        );
        assert_eq!(
            vrl_get(&value, &["request", "url", "path"]),
            &VrlValue::from("/v1/graphql")
        );
    }

    #[test]
    fn vrl_value_contains_headers() {
        let request = http::Request::builder()
            .method(Method::POST)
            .uri("http://localhost/graphql")
            .header("x-custom-header", "custom-value")
            .header("authorization", "Bearer token123")
            .body(())
            .unwrap();
        let report = make_test_report(Some("Q"), OperationType::Query, "query { hello }");
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        assert_eq!(
            vrl_get(&value, &["request", "headers", "x-custom-header"]),
            &VrlValue::from("custom-value")
        );
        assert_eq!(
            vrl_get(&value, &["request", "headers", "authorization"]),
            &VrlValue::from("Bearer token123")
        );
    }

    #[test]
    fn vrl_value_joins_duplicate_header_values() {
        let mut request = http::Request::builder()
            .method(Method::POST)
            .uri("http://localhost/graphql")
            .body(())
            .unwrap();

        request
            .headers_mut()
            .append("x-scope", http::HeaderValue::from_static("one"));
        request
            .headers_mut()
            .append("x-scope", http::HeaderValue::from_static("two"));

        let report = make_test_report(Some("Q"), OperationType::Query, "query { hello }");
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        assert_eq!(
            vrl_get(&value, &["request", "headers", "x-scope"]),
            &VrlValue::from("one, two")
        );
    }

    #[test]
    fn vrl_value_anonymous_operation_has_empty_name() {
        let report = make_simple_report(None, OperationType::Query);
        let request = make_simple_request();
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        let name = vrl_get(&value, &["request", "operation", "name"]);
        assert_eq!(name, &VrlValue::from(""));
    }

    #[test]
    fn vrl_value_has_default_false() {
        let report = make_simple_report(Some("Q"), OperationType::Query);
        let request = make_simple_request();
        let value =
            get_vrl_value_from_execution_report_and_request(&report, Some((&request).into()));

        let default_val = vrl_get(&value, &["default"]);
        assert_eq!(default_val, &VrlValue::Boolean(false));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exclude_expression_filters_by_operation_name() -> Result<(), Box<dyn std::error::Error>>
    {
        let token = "Token";
        let mut server = mockito::Server::new_async().await;
        let server_url = server.url();

        // The mock expects exactly 0 requests because the operation should be excluded
        let mock = server
            .mock("POST", "/200")
            .expect(0)
            .with_status(200)
            .create_async()
            .await;

        {
            let usage_agent = UsageAgent::builder()
                .token(token.into())
                .endpoint(format!("{}/200", server_url))
                .buffer_size(1) // flush on every report
                .exclude_expression(r#".request.operation.name == "ExcludeMe""#.to_string())
                .build()?;

            // This report should be excluded
            let report = make_simple_report(Some("ExcludeMe"), OperationType::Query);
            let request = make_simple_request();
            usage_agent
                .add_report_with_request(report, Some((&request).into()))
                .await?;
        }

        mock.assert_async().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exclude_expression_allows_non_matching_operations(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let token = "Token";
        let mut server = mockito::Server::new_async().await;
        let server_url = server.url();

        // This operation should NOT be excluded, so we expect 1 request (via async drop flush)
        let mock = server
            .mock("POST", "/200")
            .expect(1)
            .with_status(200)
            .create_async()
            .await;

        {
            let usage_agent = UsageAgent::builder()
                .token(token.into())
                .endpoint(format!("{}/200", server_url))
                .exclude_expression(r#".request.operation.name == "ExcludeMe""#.to_string())
                .build()?;

            let report = make_simple_report(Some("KeepMe"), OperationType::Query);
            let request = make_simple_request();
            usage_agent
                .add_report_with_request(report, Some((&request).into()))
                .await?;
        }

        wait_for_mock(&mock).await;
        mock.assert_async().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exclude_expression_filters_by_operation_type() -> Result<(), Box<dyn std::error::Error>>
    {
        let token = "Token";
        let mut server = mockito::Server::new_async().await;
        let server_url = server.url();

        let mock = server
            .mock("POST", "/200")
            .expect(0)
            .with_status(200)
            .create_async()
            .await;

        {
            let usage_agent = UsageAgent::builder()
                .token(token.into())
                .endpoint(format!("{}/200", server_url))
                .buffer_size(1)
                .exclude_expression(r#".request.operation.type == "subscription""#.to_string())
                .build()?;

            let report = make_simple_report(Some("OnMessage"), OperationType::Subscription);
            let request = make_simple_request();
            usage_agent
                .add_report_with_request(report, Some((&request).into()))
                .await?;
        }

        mock.assert_async().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exclude_expression_filters_by_header() -> Result<(), Box<dyn std::error::Error>> {
        let token = "Token";
        let mut server = mockito::Server::new_async().await;
        let server_url = server.url();

        let mock = server
            .mock("POST", "/200")
            .expect(0)
            .with_status(200)
            .create_async()
            .await;

        {
            let usage_agent = UsageAgent::builder()
                .token(token.into())
                .endpoint(format!("{}/200", server_url))
                .buffer_size(1)
                .exclude_expression(r#".request.headers."x-internal" == "true""#.to_string())
                .build()?;

            let request = http::Request::builder()
                .method(Method::POST)
                .uri("http://localhost/graphql")
                .header("x-internal", "true")
                .body(())
                .unwrap();

            let report = make_test_report(Some("Q"), OperationType::Query, "query { hello }");
            usage_agent
                .add_report_with_request(report, Some((&request).into()))
                .await?;
        }

        mock.assert_async().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exclude_expression_complex_conditional() -> Result<(), Box<dyn std::error::Error>> {
        let token = "Token";
        let mut server = mockito::Server::new_async().await;
        let server_url = server.url();

        // The expression excludes IntrospectionQuery OR any mutation
        let exclude_expr = r#"
            if (.request.operation.name == "IntrospectionQuery") {
                true
            } else if (.request.operation.type == "mutation") {
                true
            } else {
                false
            }
        "#;

        let mock = server
            .mock("POST", "/200")
            .expect(0)
            .with_status(200)
            .create_async()
            .await;

        {
            let usage_agent = UsageAgent::builder()
                .token(token.into())
                .endpoint(format!("{}/200", server_url))
                .buffer_size(1)
                .exclude_expression(exclude_expr.to_string())
                .build()?;

            let request = make_simple_request();

            // Excluded: IntrospectionQuery
            let report = make_simple_report(Some("IntrospectionQuery"), OperationType::Query);
            usage_agent
                .add_report_with_request(report, Some((&request).into()))
                .await?;

            // Excluded: any mutation
            let report = make_simple_report(Some("CreateUser"), OperationType::Mutation);
            usage_agent
                .add_report_with_request(report, Some((&request).into()))
                .await?;
        }

        mock.assert_async().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exclude_expression_allows_through_complex_conditional(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let token = "Token";
        let mut server = mockito::Server::new_async().await;
        let server_url = server.url();

        let exclude_expr = r#"
            if (.request.operation.name == "IntrospectionQuery") {
                true
            } else if (.request.operation.type == "mutation") {
                true
            } else {
                false
            }
        "#;

        // A normal query should NOT be excluded
        let mock = server
            .mock("POST", "/200")
            .expect(1)
            .with_status(200)
            .create_async()
            .await;

        {
            let usage_agent = UsageAgent::builder()
                .token(token.into())
                .endpoint(format!("{}/200", server_url))
                .exclude_expression(exclude_expr.to_string())
                .build()?;

            let request = make_simple_request();

            let report = make_simple_report(Some("GetUsers"), OperationType::Query);
            usage_agent
                .add_report_with_request(report, Some((&request).into()))
                .await?;
        }

        wait_for_mock(&mock).await;
        mock.assert_async().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exclude_expression_filters_by_url_path() -> Result<(), Box<dyn std::error::Error>> {
        let token = "Token";
        let mut server = mockito::Server::new_async().await;
        let server_url = server.url();

        let mock = server
            .mock("POST", "/200")
            .expect(0)
            .with_status(200)
            .create_async()
            .await;

        {
            let usage_agent = UsageAgent::builder()
                .token(token.into())
                .endpoint(format!("{}/200", server_url))
                .buffer_size(1)
                .exclude_expression(r#".request.url.path == "/internal/graphql""#.to_string())
                .build()?;

            let request = http::Request::builder()
                .method(Method::POST)
                .uri("http://localhost/internal/graphql")
                .body(())
                .unwrap();

            let report = make_test_report(Some("Q"), OperationType::Query, "query { hello }");
            usage_agent
                .add_report_with_request(report, Some((&request).into()))
                .await?;
        }

        mock.assert_async().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_exclude_expression_sends_all_reports() -> Result<(), Box<dyn std::error::Error>> {
        let token = "Token";
        let mut server = mockito::Server::new_async().await;
        let server_url = server.url();

        let mock = server
            .mock("POST", "/200")
            .expect(1)
            .with_status(200)
            .create_async()
            .await;

        {
            let usage_agent = UsageAgent::builder()
                .token(token.into())
                .endpoint(format!("{}/200", server_url))
                .build()?;

            let report = make_simple_report(Some("AnyOp"), OperationType::Query);
            let request = make_simple_request();
            usage_agent
                .add_report_with_request(report, Some((&request).into()))
                .await?;
        }

        wait_for_mock(&mock).await;
        mock.assert_async().await;
        Ok(())
    }

    #[test]
    fn builder_rejects_invalid_exclude_expression() {
        let result = UsageAgent::builder()
            .token("Token".into())
            .exclude_expression("this is not valid VRL }{".to_string())
            .build();

        assert!(result.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn builder_ignores_empty_exclude_expression() {
        let result = UsageAgent::builder()
            .token("Token".into())
            .exclude_expression("".to_string())
            .build();

        assert!(result.is_ok());
    }
}

#[cfg(test)]
mod process_variables_tests {
    use std::collections::HashSet;

    use crate::agent::utils::ReportVariables;

    use super::*;
    use graphql_tools::parser::{parse_query, parse_schema};
    use serde_json::json;

    const SCHEMA: &str = "type Query { random(a: A): String } input A { x: Int, y: Int }";
    const OPERATION: &str = "query Random($a: A) { random(a: $a) }";

    fn setup(process_variables: bool) -> (UsageAgent, Arc<Document<'static, String>>) {
        let schema = Arc::new(parse_schema::<String>(SCHEMA).unwrap());
        // the operation must parse; the agent parses it again at flush time
        parse_query::<String>(OPERATION).unwrap();
        let agent = UsageAgent::builder()
            .token("token".into())
            .process_variables(process_variables)
            .build()
            .unwrap();
        (agent, schema)
    }

    fn variables(value: serde_json::Value) -> ReportVariables {
        sonic_rs::from_str(&value.to_string()).unwrap()
    }

    fn report(
        schema: &Arc<Document<'static, String>>,
        variables: Option<ReportVariables>,
    ) -> ExecutionReport {
        ExecutionReport {
            schema: schema.clone(),
            operation_body: OPERATION.to_string(),
            operation_name: Some("Random".to_string()),
            operation_type: Some(OperationType::Query),
            ok: true,
            variables,
            ..Default::default()
        }
    }

    fn fields(report: &Report) -> HashSet<&str> {
        assert_eq!(report.map.len(), 1, "expected a single operation record");
        report
            .map
            .values()
            .next()
            .unwrap()
            .fields
            .iter()
            .map(String::as_str)
            .collect()
    }

    fn all_fields(report: &Report) -> HashSet<&str> {
        report
            .map
            .values()
            .flat_map(|record| record.fields.iter().map(String::as_str))
            .collect()
    }

    /// The operation map key covers the shape of the variables, not their values (as in the JS
    /// SDK), so executions that only differ in values share one operation record.
    #[tokio::test]
    async fn payloads_of_the_same_shape_end_up_in_the_single_operation_record() {
        let (agent, schema) = setup(true);
        assert!(agent.should_process_variables());
        let produced = agent
            .inner()
            .produce_report(vec![
                report(&schema, Some(variables(json!({ "a": { "x": 1 } })))),
                report(&schema, Some(variables(json!({ "a": { "x": 2 } })))),
            ])
            .unwrap();

        assert_eq!(produced.operations.len(), 2);
        assert_eq!(
            produced.operations[0].operation_map_key,
            produced.operations[1].operation_map_key
        );
        assert_eq!(
            fields(&produced),
            HashSet::from([
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
                "A.x",
                "A.x!",
                "Int",
            ])
        );
    }

    /// Executions with different payload shapes get their own operation record, each carrying
    /// only the input fields of its payload (as in the JS SDK).
    #[tokio::test]
    async fn payloads_of_different_shapes_get_their_own_operation_record() {
        let (agent, schema) = setup(true);
        let produced = agent
            .inner()
            .produce_report(vec![
                report(&schema, Some(variables(json!({ "a": { "x": 1 } })))),
                report(&schema, Some(variables(json!({ "a": { "y": 2 } })))),
            ])
            .unwrap();

        assert_eq!(produced.operations.len(), 2);
        assert_eq!(produced.map.len(), 2);
        assert_ne!(
            produced.operations[0].operation_map_key,
            produced.operations[1].operation_map_key
        );
        for record in produced.map.values() {
            let fields: HashSet<&str> = record.fields.iter().map(String::as_str).collect();
            assert!(
                fields.contains("A.x!") ^ fields.contains("A.y!"),
                "each record should carry exactly one payload's fields: {fields:?}"
            );
        }
        assert_eq!(
            all_fields(&produced),
            HashSet::from([
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
                "A.x",
                "A.x!",
                "A.y",
                "A.y!",
                "Int",
            ])
        );
    }

    #[tokio::test]
    async fn executions_without_a_payload_report_coarse_coordinates() {
        let (agent, schema) = setup(true);
        let produced = agent
            .inner()
            .produce_report(vec![report(&schema, None)])
            .unwrap();

        assert_eq!(
            fields(&produced),
            HashSet::from(["Query.random", "Query.random.a", "A.x", "A.y", "Int"])
        );
    }

    #[tokio::test]
    async fn payloads_are_ignored_when_process_variables_is_disabled() {
        let (agent, schema) = setup(false);
        assert!(!agent.should_process_variables());
        let produced = agent
            .inner()
            .produce_report(vec![
                report(&schema, Some(variables(json!({ "a": { "x": 1 } })))),
                report(&schema, Some(variables(json!({ "a": { "y": 2 } })))),
            ])
            .unwrap();

        assert_eq!(
            fields(&produced),
            HashSet::from(["Query.random", "Query.random.a", "A.x", "A.y", "Int"])
        );
    }
}

/// 1:1 port of `collect-schema-coordinates.spec.ts` from `@graphql-hive/core`
#[cfg(test)]
mod js_parity_tests {
    use super::*;
    use crate::agent::utils::ReportVariables;
    use graphql_tools::parser::parse_schema;
    use serde_json::{json, Value};

    /// Mirrors `collectSchemaCoordinates({ documentNode, schema, processVariables, variables })`:
    /// the report's `fields` for one execution of `operation` with `variables`.
    async fn collect(
        schema: &'static str,
        operation: &str,
        process_variables: bool,
        variables: Option<Value>,
    ) -> Vec<String> {
        let schema = Arc::new(parse_schema::<String>(schema).unwrap());
        let agent = UsageAgent::builder()
            .token("token".into())
            .process_variables(process_variables)
            .build()
            .unwrap();
        let variables: Option<ReportVariables> =
            variables.map(|value| sonic_rs::from_str(&value.to_string()).unwrap());
        let produced = agent
            .inner()
            .produce_report(vec![ExecutionReport {
                schema,
                operation_body: operation.to_string(),
                operation_type: Some(OperationType::Query),
                ok: true,
                variables,
                ..Default::default()
            }])
            .unwrap();
        let record = produced
            .map
            .values()
            .next()
            .expect("the operation should be reported");
        let mut fields = record.fields.clone();
        fields.sort();
        fields
    }

    fn assert_coordinates(actual: Vec<String>, expected: &[&str]) {
        let mut expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(actual, expected);
    }

    const NESTED: &str = r#"
        type Query { random(a: A): String }
        input A { b: B }
        input B { c: C }
        input C { d: String }
    "#;

    const PROJECTS: &str = r#"
        type Query {
            project(selector: ProjectSelectorInput!): Project
            projectsByType(type: ProjectType!): [Project!]!
            projectsByTypes(types: [ProjectType!]!): [Project!]!
            projects(filter: FilterInput, and: [FilterInput!]): [Project!]!
            projectsByMetadata(metadata: JSON): [Project!]!
        }
        type Mutation { deleteProject(selector: ProjectSelectorInput!): DeleteProjectPayload! }
        input ProjectSelectorInput { organization: ID!, project: ID! }
        input FilterInput { type: ProjectType, pagination: PaginationInput, order: [ProjectOrderByInput!], metadata: JSON }
        input PaginationInput { limit: Int, offset: Int }
        input ProjectOrderByInput { field: String!, direction: OrderDirection }
        enum OrderDirection { ASC DESC }
        type ProjectSelector { organization: ID!, project: ID! }
        type DeleteProjectPayload { selector: ProjectSelector!, deletedProject: Project! }
        type Project { id: ID!, cleanId: ID!, name: String!, type: ProjectType!, buildUrl: String, validationUrl: String }
        enum ProjectType { FEDERATION STITCHING SINGLE }
        scalar JSON
    "#;

    const NESTED_FRAGMENT_OP: &str = r#"
        query getProjects($limit: Int!, $type: ProjectType!, $includeName: Boolean!) {
            projects(filter: { pagination: { limit: $limit }, type: $type }) {
                id
                ...NestedFragment
            }
        }
        fragment NestedFragment on Project { ...IncludeNameFragment @include(if: $includeName) }
        fragment IncludeNameFragment on Project { name }
    "#;

    const NODE: &str = r#"
        type Query { node: Node }
        interface Node { id: ID! }
        type User implements Node { id: ID! }
        type Animal implements Node { id: ID! }
    "#;

    #[tokio::test]
    async fn single_primitive_field_schema_coordinate() {
        let result = collect(
            "type Query { hello: String }",
            "query { hello }",
            false,
            None,
        )
        .await;
        assert_coordinates(result, &["Query.hello"]);
    }

    #[tokio::test]
    async fn two_primitive_field_schema_coordinates() {
        let result = collect(
            "type Query { hello: String, hi: String }",
            "query { hello hi }",
            false,
            None,
        )
        .await;
        assert_coordinates(result, &["Query.hello", "Query.hi"]);
    }

    #[tokio::test]
    async fn primitive_field_with_arguments_schema_coordinates() {
        let result = collect(
            "type Query { hello(message: String): String }",
            r#"query { hello(message: "world") }"#,
            false,
            None,
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.hello",
                "Query.hello.message!",
                "Query.hello.message",
                "String",
            ],
        );
    }

    #[tokio::test]
    async fn leaf_field_enum() {
        let result = collect(
            "type Query { hello: Option } enum Option { World You }",
            "query { hello }",
            false,
            None,
        )
        .await;
        assert_coordinates(result, &["Query.hello", "Option.World", "Option.You"]);
    }

    #[tokio::test]
    async fn interface_selection_set_does_not_contain_exact_resolutions() {
        let result = collect(NODE, "query { node { id } }", false, None).await;
        assert_coordinates(result, &["Query.node", "Node.id"]);
    }

    #[tokio::test]
    async fn inline_fragment_spread_contains_exact_resolutions() {
        let result = collect(
            NODE,
            "query { node { id ... on User { id } } }",
            false,
            None,
        )
        .await;
        assert_coordinates(result, &["Query.node", "Node.id", "User.id"]);
    }

    #[tokio::test]
    async fn custom_scalar_as_argument() {
        let result = collect(
            "type Query { random(json: JSON): String } scalar JSON",
            r#"query { random(json: { key: { value: "value" } }) }"#,
            false,
            None,
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.json",
                "Query.random.json!",
                "JSON",
            ],
        );
    }

    #[tokio::test]
    async fn custom_scalar_in_input_object_field() {
        let result = collect(
            "type Query { random(input: I): String } input I { json: JSON } scalar JSON",
            r#"query { random(input: { json: { key: { value: "value" } } }) }"#,
            false,
            None,
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.input",
                "Query.random.input!",
                "I.json",
                "I.json!",
                "JSON",
            ],
        );
    }

    #[tokio::test]
    async fn deeply_nested_inputs() {
        let result = collect(
            NESTED,
            r#"query { random(a: { b: { c: { d: "D" } } }) }"#,
            false,
            None,
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
                "A.b",
                "A.b!",
                "B.c",
                "B.c!",
                "C.d",
                "C.d!",
                "String",
            ],
        );
    }

    #[tokio::test]
    async fn required_variable_as_argument() {
        let result = collect(
            "type Query { random(a: String): String }",
            "query Foo($a: String!) { random(a: $a) }",
            true,
            Some(json!({ "a": "B" })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
                "String",
            ],
        );
    }

    #[tokio::test]
    async fn unused_variable_as_nullable_argument() {
        let result = collect(
            "type Query { random(a: String): String }",
            "query Foo($a: String) { random(a: $a) }",
            true,
            Some(json!({})),
        )
        .await;
        assert_coordinates(result, &["Query.random", "Query.random.a", "String"]);
    }

    #[tokio::test]
    async fn unused_nullable_argument() {
        let result = collect(
            "type Query { random(a: String): String }",
            "query Foo { random }",
            true,
            None,
        )
        .await;
        assert_coordinates(result, &["Query.random"]);
    }

    #[tokio::test]
    async fn unused_nullable_input_field() {
        let result = collect(NESTED, "query Foo { random(a: { b: null }) }", true, None).await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
                "A.b",
                "B.c",
                "C.d",
                "String",
            ],
        );
    }

    #[tokio::test]
    async fn required_variable_as_input_field() {
        let result = collect(
            "type Query { random(a: A): String } input A { b: String }",
            "query Foo($b: String!) { random(a: { b: $b }) }",
            true,
            Some(json!({ "b": "B" })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
                "A.b",
                "A.b!",
                "String",
            ],
        );
    }

    /// `processVariables=true` with no payload falls back to the coarse collection.
    #[tokio::test]
    async fn undefined_variable_as_input_field() {
        let result = collect(
            "type Query { random(a: A): String } input A { b: String }",
            "query Foo($b: String!) { random(a: { b: $b }) }",
            true,
            None,
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
                "A.b",
                "String",
            ],
        );
    }

    #[tokio::test]
    async fn deeply_nested_variables_process_variables_true() {
        let result = collect(
            NESTED,
            "query Random($a: A) { random(a: $a) }",
            true,
            Some(json!({ "a": { "b": { "c": { "d": "D" } } } })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
                "A.b",
                "A.b!",
                "B.c",
                "B.c!",
                "C.d",
                "C.d!",
                "String",
            ],
        );
    }

    /// Deviation from the JS spec: with `processVariables=false` the JS SDK still reads the
    /// runtime payload for the `Query.random.a!` marker. With the feature off, the Rust SDK
    /// never looks at the payload, so the marker is not reported.
    #[tokio::test]
    async fn deeply_nested_variables_process_variables_false() {
        let result = collect(
            NESTED,
            "query Random($a: A) { random(a: $a) }",
            false,
            Some(json!({ "a": { "b": { "c": { "d": "D" } } } })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.a",
                "A.b",
                "B.c",
                "C.d",
                "String",
            ],
        );
    }

    #[tokio::test]
    async fn aliased_field() {
        let result = collect(
            "type Query { random(a: String): String } input C { d: String }",
            "query Random($a: String) { foo: random(a: $a) }",
            true,
            Some(json!({ "a": "B" })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
                "String",
            ],
        );
    }

    #[tokio::test]
    async fn multiple_fields_with_mixed_nullability() {
        let result = collect(
            "type Query { random(a: String): String } input C { d: String }",
            r#"query Random($a: String) { nullable: random(a: $a) nonnullable: random(a: "B") }"#,
            false,
            Some(json!({ "a": null })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
                "String",
            ],
        );
    }

    #[tokio::test]
    async fn nested_fragment_with_client_side_directive() {
        let result = collect(
            PROJECTS,
            NESTED_FRAGMENT_OP,
            false,
            Some(json!({ "includeName": true })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Boolean",
                "FilterInput.pagination",
                "FilterInput.pagination!",
                "FilterInput.type",
                "Int",
                "PaginationInput.limit",
                "Project.id",
                "Project.name",
                "ProjectType.FEDERATION",
                "ProjectType.SINGLE",
                "ProjectType.STITCHING",
                "Query.projects",
                "Query.projects.filter",
                "Query.projects.filter!",
            ],
        );
    }

    /// Variables referenced inside literal input objects get their `!` from the payload.
    #[tokio::test]
    async fn granular_marks_variable_positions_inside_literals_from_the_payload() {
        let result = collect(
            PROJECTS,
            NESTED_FRAGMENT_OP,
            true,
            Some(json!({ "limit": 10, "type": "FEDERATION", "includeName": true })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Boolean",
                "FilterInput.pagination",
                "FilterInput.pagination!",
                "FilterInput.type",
                "FilterInput.type!",
                "Int",
                "PaginationInput.limit",
                "PaginationInput.limit!",
                "Project.id",
                "Project.name",
                "ProjectType.FEDERATION",
                "ProjectType.SINGLE",
                "ProjectType.STITCHING",
                "Query.projects",
                "Query.projects.filter",
                "Query.projects.filter!",
            ],
        );
    }

    /// JS `collectVariable`: an absent (or null) input-object variable is reported as its bare
    /// type name, without `!` on the argument.
    #[tokio::test]
    async fn granular_absent_input_object_variable_marks_only_the_bare_type() {
        let result = collect(
            NESTED,
            "query Random($a: A) { random(a: $a) }",
            true,
            Some(json!({})),
        )
        .await;
        assert_coordinates(result, &["Query.random", "Query.random.a", "A"]);
    }

    #[tokio::test]
    async fn granular_null_nested_input_object_marks_its_bare_type() {
        let result = collect(
            NESTED,
            "query Random($a: A) { random(a: $a) }",
            true,
            Some(json!({ "a": { "b": null } })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "A.b",
                "B",
                "Query.random",
                "Query.random.a",
                "Query.random.a!",
            ],
        );
    }

    #[tokio::test]
    async fn granular_list_of_input_objects_reports_only_present_fields() {
        let result = collect(
            PROJECTS,
            "query Q($and: [FilterInput!]) { projects(and: $and) { id } }",
            true,
            Some(json!({ "and": [{ "type": "SINGLE" }, { "pagination": { "offset": 2 } }] })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "FilterInput.pagination",
                "FilterInput.pagination!",
                "FilterInput.type",
                "FilterInput.type!",
                "Int",
                "PaginationInput.offset",
                "PaginationInput.offset!",
                "Project.id",
                "ProjectType.FEDERATION",
                "ProjectType.SINGLE",
                "ProjectType.STITCHING",
                "Query.projects",
                "Query.projects.and",
                "Query.projects.and!",
            ],
        );
    }

    #[tokio::test]
    async fn granular_recursive_input_types() {
        let result = collect(
            "type Query { random(f: Filter): String } input Filter { and: [Filter], name: String }",
            "query Random($f: Filter) { random(f: $f) }",
            true,
            Some(json!({ "f": { "and": [{ "and": [{ "name": "x" }] }] } })),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "Filter.and",
                "Filter.and!",
                "Filter.name",
                "Filter.name!",
                "Query.random",
                "Query.random.f",
                "Query.random.f!",
                "String",
            ],
        );
    }

    /// As in the JS SDK, default values are not applied for absent variables: the default
    /// literal is collected by the visitor like any other literal (graphql-js `visit` walks
    /// `VariableDefinition.defaultValue`), and the absent variable itself is reported as its
    /// bare type, without a `!` marker on the argument.
    #[tokio::test]
    async fn granular_absent_variable_with_default_follows_the_js_sdk() {
        let result = collect(
            PROJECTS,
            r#"query Q($filter: FilterInput = { type: SINGLE }) { projects(filter: $filter) { id } }"#,
            true,
            Some(json!({})),
        )
        .await;
        assert_coordinates(
            result,
            &[
                "FilterInput",
                "FilterInput.type",
                "FilterInput.type!",
                "Project.id",
                "ProjectType.SINGLE",
                "Query.projects",
                "Query.projects.filter",
            ],
        );
    }

    /// The JS SDK strips directive arguments before collecting, so `@include(if: $x)` on a field
    /// must not produce a `Query.users.if` coordinate.
    #[tokio::test]
    async fn directive_arguments_on_fields_are_not_coordinates() {
        for process_variables in [false, true] {
            let result = collect(
                "type Query { users: [User] } type User { id: ID }",
                "query Q($x: Boolean!) { users @include(if: $x) { id @skip(if: false) } }",
                process_variables,
                Some(json!({ "x": true })),
            )
            .await;
            assert_coordinates(result, &["Boolean", "Query.users", "User.id"]);
        }
    }
}
