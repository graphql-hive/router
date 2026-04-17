use async_dropper_simple::{AsyncDrop, AsyncDropper};
use graphql_tools::parser::schema::Document;
use recloser::AsyncRecloser;
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::expressions::lib::FromVrlValue;
use crate::{
    agent::{buffer::AddStatus, utils::OperationProcessor},
    expressions::ExecutableProgram,
};
use crate::{
    agent::{buffer::Buffer, builder::UsageAgentBuilder},
    expressions::values::boolean::{BooleanConversionError, BooleanOrProgram},
};
use vrl::{core::Value as VrlValue, value::KeyString};

#[derive(Debug, Clone, Default)]
pub enum OperationType {
    #[default]
    Query,
    Mutation,
    Subscription,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestDetails {
    pub headers: BTreeMap<String, String>,
    pub method: String,
    pub url: RequestDetailsUrl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestDetailsUrl {
    pub host: String,
    pub port: u16,
    pub path: String,
}

#[derive(Debug, Clone)]
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
    pub operation_type: OperationType,
    pub persisted_document_hash: Option<String>,
    pub request_details: RequestDetails,
}

typify::import_types!(schema = "./usage-report-v2.schema.json");

pub struct UsageAgentInner {
    pub(crate) endpoint: String,
    pub(crate) buffer: Buffer<ExecutionReport>,
    pub(crate) processor: OperationProcessor,
    pub(crate) client: ClientWithMiddleware,
    pub(crate) flush_interval: Duration,
    pub(crate) circuit_breaker: AsyncRecloser,
    pub(crate) exclude: BooleanOrProgram,
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
    #[error("invalid target id provided: {0}, it should be either a slug like \"$organizationSlug/$projectSlug/$targetSlug\" or an UUID")]
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

pub type UsageAgent = Arc<AsyncDropper<UsageAgentInner>>;

#[async_trait::async_trait]
pub trait UsageAgentExt {
    fn builder() -> UsageAgentBuilder {
        UsageAgentBuilder::default()
    }
    async fn flush(&self) -> Result<(), AgentError>;
    async fn start_flush_interval(&self, token: &CancellationToken);
    async fn add_report(&self, execution_report: ExecutionReport) -> Result<(), AgentError>;
}

impl UsageAgentInner {
    fn produce_report(&self, reports: Vec<ExecutionReport>) -> Result<Report, AgentError> {
        let mut report = Report {
            size: 0,
            map: HashMap::new(),
            operations: Vec::new(),
            subscription_operations: Vec::new(),
        };

        // iterate over reports and check if they are valid
        for op in reports {
            let operation = self.processor.process(&op.operation_body, &op.schema);
            match operation {
                Err(e) => {
                    tracing::warn!(
                        "Dropping operation \"{}\" (phase: PROCESSING): {}",
                        op.operation_name
                            .clone()
                            .or_else(|| Some("anonymous".to_string()))
                            .unwrap(),
                        e
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
                            "Dropping operation (phase: PROCESSING): probably introspection query"
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

#[async_trait::async_trait]
impl UsageAgentExt for UsageAgent {
    async fn flush(&self) -> Result<(), AgentError> {
        self.inner().flush().await
    }

    async fn start_flush_interval(&self, token: &CancellationToken) {
        loop {
            tokio::time::sleep(self.inner().flush_interval).await;
            if token.is_cancelled() {
                println!("Shutting down.");
                return;
            }
            self.flush()
                .await
                .unwrap_or_else(|e| tracing::error!("Failed to flush usage reports: {}", e));
        }
    }

    async fn add_report(&self, execution_report: ExecutionReport) -> Result<(), AgentError> {
        let inner = self.inner();
        if let BooleanOrProgram::Program(exclude_program) = &inner.exclude {
            let result =
                exclude_program.execute(get_vrl_value_from_execution_report(&execution_report))?;
            let result_bool = bool::from_vrl_value(result)?;
            if result_bool {
                tracing::debug!(
                    "Excluding report for operation \"{}\" based on exclude expression evaluation",
                    execution_report
                        .operation_name
                        .clone()
                        .or_else(|| Some("anonymous".to_string()))
                        .unwrap()
                );
                return Ok(());
            }
        }
        if let AddStatus::Full { drained } = inner.buffer.add(execution_report).await {
            inner.handle_drained(drained).await?;
        }

        Ok(())
    }
}

pub fn get_vrl_value_from_execution_report(report: &ExecutionReport) -> VrlValue {
    let mut headers_value: BTreeMap<KeyString, VrlValue> = BTreeMap::new();
    for (header_name, header_value) in report.request_details.headers.iter() {
        headers_value.insert(header_name.as_str().into(), header_value.as_str().into());
    }

    let headers_value = VrlValue::Object(headers_value);

    // .request.url
    let url_value = VrlValue::Object(BTreeMap::from([
        (
            "host".into(),
            report.request_details.url.host.as_str().into(),
        ),
        (
            "path".into(),
            report.request_details.url.path.as_str().into(),
        ),
        (
            "port".into(),
            VrlValue::Integer(report.request_details.url.port.into()),
        ),
    ]));

    // .request.operation
    let operation_value = VrlValue::Object(BTreeMap::from([
        (
            "name".into(),
            report.operation_name.as_deref().unwrap_or_default().into(),
        ),
        (
            "type".into(),
            match report.operation_type {
                OperationType::Query => "query".into(),
                OperationType::Mutation => "mutation".into(),
                OperationType::Subscription => "subscription".into(),
            },
        ),
        ("query".into(), report.operation_body.as_str().into()),
    ]));

    // .request
    let request_value = VrlValue::Object(BTreeMap::from([
        (
            "method".into(),
            report.request_details.method.as_str().into(),
        ),
        ("headers".into(), headers_value),
        ("url".into(), url_value),
        ("operation".into(), operation_value),
    ]));

    VrlValue::Object(BTreeMap::from([
        ("request".into(), request_value),
        ("default".into(), VrlValue::Boolean(false)),
    ]))
}

#[async_trait::async_trait]
impl AsyncDrop for UsageAgentInner {
    async fn async_drop(&mut self) {
        if let Err(e) = self.flush().await {
            tracing::error!("Failed to flush usage reports during drop: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc, time::Duration};

    use graphql_tools::parser::{parse_query, parse_schema};
    use reqwest::{
        header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE, USER_AGENT},
        Method,
    };
    use vrl::core::Value as VrlValue;
    use vrl::value::KeyString;

    use crate::agent::usage_agent::{
        get_vrl_value_from_execution_report, ExecutionReport, OperationType, Report,
        RequestDetails, RequestDetailsUrl, UsageAgent, UsageAgentExt,
    };

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

            usage_agent
                .add_report(ExecutionReport {
                    schema: Arc::new(schema),
                    operation_body: op.to_string(),
                    operation_name: Some("deleteProject".to_string()),
                    operation_type: OperationType::Mutation,
                    client_name: Some(GRAPHQL_CLIENT_NAME.to_string()),
                    client_version: Some(GRAPHQL_CLIENT_VERSION.to_string()),
                    timestamp,
                    duration,
                    ok: true,
                    errors: 0,
                    persisted_document_hash: None,
                    request_details: RequestDetails {
                        headers: Default::default(),
                        method: "POST".to_string(),
                        url: RequestDetailsUrl {
                            host: "example.com".to_string(),
                            port: 443,
                            path: "/graphql".to_string(),
                        },
                    },
                })
                .await?;
        }

        mock.assert_async().await;

        Ok(())
    }

    fn make_test_report(
        operation_name: Option<&str>,
        operation_type: OperationType,
        method: &str,
        headers: BTreeMap<String, String>,
        host: &str,
        port: u16,
        path: &str,
        operation_body: &str,
    ) -> ExecutionReport {
        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema("type Query { hello: String }").unwrap();

        ExecutionReport {
            schema: Arc::new(schema),
            operation_body: operation_body.to_string(),
            operation_name: operation_name.map(|s| s.to_string()),
            operation_type,
            client_name: Some("test-client".to_string()),
            client_version: Some("1.0.0".to_string()),
            timestamp: 1625247600,
            duration: Duration::from_millis(10),
            ok: true,
            errors: 0,
            persisted_document_hash: None,
            request_details: RequestDetails {
                headers,
                method: method.to_string(),
                url: RequestDetailsUrl {
                    host: host.to_string(),
                    port,
                    path: path.to_string(),
                },
            },
        }
    }

    fn make_simple_report(
        operation_name: Option<&str>,
        operation_type: OperationType,
    ) -> ExecutionReport {
        make_test_report(
            operation_name,
            operation_type,
            "POST",
            BTreeMap::new(),
            "localhost",
            80,
            "/graphql",
            "query { hello }",
        )
    }

    #[test]
    fn vrl_value_contains_operation_name() {
        let report = make_simple_report(Some("MyQuery"), OperationType::Query);
        let value = get_vrl_value_from_execution_report(&report);

        let name = vrl_get(&value, &["request", "operation", "name"]);
        assert_eq!(name, &VrlValue::from("MyQuery"));
    }

    #[test]
    fn vrl_value_contains_operation_type_query() {
        let report = make_simple_report(Some("Q"), OperationType::Query);
        let value = get_vrl_value_from_execution_report(&report);

        let op_type = vrl_get(&value, &["request", "operation", "type"]);
        assert_eq!(op_type, &VrlValue::from("query"));
    }

    #[test]
    fn vrl_value_contains_operation_type_mutation() {
        let report = make_simple_report(Some("M"), OperationType::Mutation);
        let value = get_vrl_value_from_execution_report(&report);

        let op_type = vrl_get(&value, &["request", "operation", "type"]);
        assert_eq!(op_type, &VrlValue::from("mutation"));
    }

    #[test]
    fn vrl_value_contains_operation_type_subscription() {
        let report = make_simple_report(Some("S"), OperationType::Subscription);
        let value = get_vrl_value_from_execution_report(&report);

        let op_type = vrl_get(&value, &["request", "operation", "type"]);
        assert_eq!(op_type, &VrlValue::from("subscription"));
    }

    #[test]
    fn vrl_value_contains_operation_body() {
        let report = make_simple_report(Some("Q"), OperationType::Query);
        let value = get_vrl_value_from_execution_report(&report);

        let query = vrl_get(&value, &["request", "operation", "query"]);
        assert_eq!(query, &VrlValue::from("query { hello }"));
    }

    #[test]
    fn vrl_value_contains_request_method() {
        let report = make_test_report(
            Some("Q"),
            OperationType::Query,
            "GET",
            BTreeMap::new(),
            "localhost",
            80,
            "/graphql",
            "query { hello }",
        );
        let value = get_vrl_value_from_execution_report(&report);

        let method = vrl_get(&value, &["request", "method"]);
        assert_eq!(method, &VrlValue::from("GET"));
    }

    #[test]
    fn vrl_value_contains_url_details() {
        let report = make_test_report(
            Some("Q"),
            OperationType::Query,
            "POST",
            BTreeMap::new(),
            "api.example.com",
            8080,
            "/v1/graphql",
            "query { hello }",
        );
        let value = get_vrl_value_from_execution_report(&report);

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
        let mut headers = BTreeMap::new();
        headers.insert("x-custom-header".to_string(), "custom-value".to_string());
        headers.insert("authorization".to_string(), "Bearer token123".to_string());

        let report = make_test_report(
            Some("Q"),
            OperationType::Query,
            "POST",
            headers,
            "localhost",
            80,
            "/graphql",
            "query { hello }",
        );
        let value = get_vrl_value_from_execution_report(&report);

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
    fn vrl_value_anonymous_operation_has_empty_name() {
        let report = make_simple_report(None, OperationType::Query);
        let value = get_vrl_value_from_execution_report(&report);

        let name = vrl_get(&value, &["request", "operation", "name"]);
        assert_eq!(name, &VrlValue::from(""));
    }

    #[test]
    fn vrl_value_has_default_false() {
        let report = make_simple_report(Some("Q"), OperationType::Query);
        let value = get_vrl_value_from_execution_report(&report);

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
            usage_agent.add_report(report).await?;
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
            usage_agent.add_report(report).await?;
        }

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
            usage_agent.add_report(report).await?;
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

            let mut headers = BTreeMap::new();
            headers.insert("x-internal".to_string(), "true".to_string());
            let report = make_test_report(
                Some("Q"),
                OperationType::Query,
                "POST",
                headers,
                "localhost",
                80,
                "/graphql",
                "query { hello }",
            );
            usage_agent.add_report(report).await?;
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

            // Excluded: IntrospectionQuery
            let report = make_simple_report(Some("IntrospectionQuery"), OperationType::Query);
            usage_agent.add_report(report).await?;

            // Excluded: any mutation
            let report = make_simple_report(Some("CreateUser"), OperationType::Mutation);
            usage_agent.add_report(report).await?;
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

            let report = make_simple_report(Some("GetUsers"), OperationType::Query);
            usage_agent.add_report(report).await?;
        }

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

            let report = make_test_report(
                Some("Q"),
                OperationType::Query,
                "POST",
                BTreeMap::new(),
                "localhost",
                80,
                "/internal/graphql",
                "query { hello }",
            );
            usage_agent.add_report(report).await?;
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
            usage_agent.add_report(report).await?;
        }

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
