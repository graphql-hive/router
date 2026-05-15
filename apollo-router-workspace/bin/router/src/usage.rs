use crate::consts::PLUGIN_VERSION;
use apollo_router::Context;
use apollo_router::layers::ServiceBuilderExt;
use apollo_router::plugin::Plugin;
use apollo_router::plugin::PluginInit;
use apollo_router::services::*;
use core::ops::Drop;
use futures::StreamExt;
use hive_console_sdk::agent::config::UsageReportingConfig;
use hive_console_sdk::agent::usage_agent::RequestDetails;
use hive_console_sdk::agent::usage_agent::UsageAgentExt;
use hive_console_sdk::agent::usage_agent::{ExecutionReport, UsageAgent};
use hive_console_sdk::graphql_tools::parser::parse_schema;
use hive_console_sdk::graphql_tools::parser::schema::Document;
use hive_console_sdk::primitives::target_id::TargetId;
use http::HeaderValue;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;
use tower::BoxError;
use tower::ServiceBuilder;
use tower::ServiceExt;

use crate::persisted_documents::PERSISTED_DOCUMENT_HASH_KEY;

pub(crate) static OPERATION_CONTEXT: &str = "hive::operation_context";

#[derive(Serialize, Deserialize, Debug)]
struct OperationContext {
    pub(crate) client_name: Option<String>,
    pub(crate) client_version: Option<String>,
    pub(crate) timestamp: u64,
    pub(crate) operation_body: String,
    pub(crate) operation_name: Option<String>,
}

#[derive(Clone, Debug)]
struct OperationConfig {
    client_name_header: String,
    client_version_header: String,
}

pub struct UsagePlugin {
    config: OperationConfig,
    agent: Option<UsageAgent>,
    schema: Arc<Document<'static, String>>,
    cancellation_token: Arc<CancellationToken>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
pub struct Config {
    /// Hive token, can also be set using the `HIVE_TOKEN` environment variable.
    /// The token can be a registry access token or an organization access token.
    token: Option<String>,
    /// The target to which the usage data should be reported to.
    /// This can either be a slug following the format "$organizationSlug/$projectSlug/$targetSlug" (e.g "the-guild/graphql-hive/staging")
    /// or a UUID (e.g. "a0f4c605-6541-4350-8cfe-b31f21a4bf80").
    /// Can also be set using the `HIVE_TARGET_ID` environment variable.
    target: Option<TargetId>,
    /// HTTP header used to identify the GraphQL client name.
    /// Default: `graphql-client-name`.
    client_name_header: Option<String>,
    /// HTTP header used to identify the GraphQL client version.
    /// Default: `graphql-client-version`.
    client_version_header: Option<String>,
    /// All Hive Console usage-reporting settings (endpoint, sampler,
    /// exclude, buffer size, timeouts, ...). Re-exported from
    /// [`hive_console_sdk::agent::config::UsageReportingConfig`] so the
    /// same shape is shared with `hive-router`.
    #[serde(flatten)]
    reporting: UsageReportingConfig,
}

impl UsagePlugin {
    fn populate_context(config: OperationConfig, req: &supergraph::Request) {
        let context = &req.context;
        let http_request = &req.supergraph_request;
        let headers = http_request.headers();

        let get_header_value = |key: &str| {
            headers
                .get(key)
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_static(""))
                .to_str()
                .ok()
                .map(|v| v.to_string())
        };

        let client_name = get_header_value(&config.client_name_header);
        let client_version = get_header_value(&config.client_version_header);

        let operation_name = req.supergraph_request.body().operation_name.clone();
        let operation_body = req
            .supergraph_request
            .body()
            .query
            .clone()
            .unwrap_or_default();

        let _ = context.insert(
            OPERATION_CONTEXT,
            OperationContext {
                client_name,
                client_version,
                operation_name,
                operation_body,
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    * 1000,
            },
        );
    }
}

#[async_trait::async_trait]
impl Plugin for UsagePlugin {
    type Config = Config;

    async fn new(init: PluginInit<Config>) -> Result<Self, BoxError> {
        let mut user_config = init.config;

        let enabled = user_config.reporting.enabled;

        if enabled {
            tracing::info!("Starting GraphQL Hive Usage plugin");
        }

        // Allow `HIVE_ENDPOINT` env var to override the default endpoint
        // when the operator has not set `endpoint` in the YAML config.
        if user_config.reporting.endpoint
            == hive_console_sdk::agent::config::DEFAULT_HIVE_USAGE_ENDPOINT
        {
            if let Ok(env_endpoint) = env::var("HIVE_ENDPOINT") {
                user_config.reporting.endpoint = env_endpoint;
            }
        }

        let token = user_config.token.or_else(|| env::var("HIVE_TOKEN").ok());
        let target: Option<TargetId> = match user_config.target {
            Some(t) => Some(t),
            None => match env::var("HIVE_TARGET_ID") {
                Ok(raw) => Some(TargetId::parse(raw).map_err(Box::new)?),
                Err(_) => None,
            },
        };

        let cancellation_token = Arc::new(CancellationToken::new());

        let agent = if enabled {
            let mut builder = UsageAgent::builder()
                .user_agent(format!("hive-apollo-router/{}", PLUGIN_VERSION))
                .from_config(&user_config.reporting)?;

            if let Some(token) = token {
                builder = builder.token(token);
            }
            if let Some(target_id) = target {
                builder = builder.target_id(target_id);
            }

            let agent = builder.build().map_err(Box::new)?;

            let cancellation_token_for_interval = cancellation_token.clone();
            let agent_for_interval = agent.clone();
            tokio::task::spawn(async move {
                agent_for_interval
                    .start_flush_interval(&cancellation_token_for_interval)
                    .await;
            });
            Some(agent)
        } else {
            None
        };

        let schema = parse_schema(&init.supergraph_sdl)
            .expect("Failed to parse schema")
            .into_static();

        Ok(UsagePlugin {
            schema: Arc::new(schema),
            config: OperationConfig {
                client_name_header: user_config
                    .client_name_header
                    .unwrap_or("graphql-client-name".to_string()),
                client_version_header: user_config
                    .client_version_header
                    .unwrap_or("graphql-client-version".to_string()),
            },
            agent,
            cancellation_token,
        })
    }

    fn supergraph_service(&self, service: supergraph::BoxService) -> supergraph::BoxService {
        let config = self.config.clone();
        let schema = self.schema.clone();
        match self.agent.clone() {
            None => ServiceBuilder::new().service(service).boxed(),
            Some(agent) => {
                ServiceBuilder::new()
                    .map_future_with_request_data(
                        move |req: &supergraph::Request| {
                            Self::populate_context(config.clone(), req);

                            let request_details = RequestDetails {
                                method: req.supergraph_request.method().clone(),
                                url: req.supergraph_request.uri().clone(),
                                headers: req
                                    .supergraph_request
                                    .headers()
                                    .iter()
                                    .filter_map(|(k, v)| {
                                        v.to_str()
                                            .ok()
                                            .map(|value| (k.to_string(), value.to_string()))
                                    })
                                    .collect(),
                            };
                            (request_details, req.context.clone())
                        },
                        move |(request_details, ctx): (RequestDetails, Context), fut| {
                            let agent = agent.clone();
                            let schema = schema.clone();
                            async move {
                                let start: Instant = Instant::now();

                                let result: supergraph::ServiceResult = fut.await;

                                // nested async block, bc async is unstable with closures that receive arguments
                                let Some(operation_context) = ctx
                                    .get::<_, OperationContext>(OPERATION_CONTEXT)
                                    .unwrap_or_default() else {
                                    tracing::debug!("Operation context not found in request context, skipping usage reporting");
                                    return result;
                                };

                                // Injected by the persisted document plugin, if it was activated
                                // and discovered document id
                                let persisted_document_hash = ctx
                                    .get::<_, String>(PERSISTED_DOCUMENT_HASH_KEY)
                                    .unwrap_or_default();

                                let OperationContext {
                                    client_name,
                                    client_version,
                                    operation_name,
                                    timestamp,
                                    operation_body,
                                } = operation_context;

                                let duration = start.elapsed();

                                match result {
                                    Err(e) => {
                                        tokio::spawn(async move {
                                            let res = agent
                                                .add_report_with_request(ExecutionReport {
                                                    schema,
                                                    client_name,
                                                    client_version,
                                                    timestamp,
                                                    duration,
                                                    errors: 1,
                                                    operation_body,
                                                    operation_name,
                                                    persisted_document_hash,
                                                    ..Default::default()
                                                }, Some(request_details))
                                                .await;
                                            if let Err(e) = res {
                                                tracing::error!("Error adding report: {}", e);
                                            }
                                        });
                                        Err(e)
                                    }
                                    Ok(router_response) => {
                                        let is_failure =
                                            !router_response.response.status().is_success();
                                        Ok(router_response.map(move |response_stream| {
                                            let res = response_stream
                                                .map(move |response| {
                                                    // make sure we send a single report, not for each chunk
                                                    let response_has_errors =
                                                        !response.errors.is_empty();
                                                    let agent = agent.clone();
                                                    let execution_report = ExecutionReport {
                                                        schema: schema.clone(),
                                                        client_name: client_name.clone(),
                                                        client_version: client_version.clone(),
                                                        timestamp,
                                                        duration,
                                                        ok: !is_failure && !response_has_errors,
                                                        errors: response.errors.len(),
                                                        operation_body: operation_body.clone(),
                                                        operation_name: operation_name.clone(),
                                                        persisted_document_hash: persisted_document_hash.clone(),
                                                        ..Default::default()
                                                    };
                                                    let request_details = request_details.clone();
                                                    tokio::spawn(async move {
                                                        let res = agent
                                                            .add_report_with_request(execution_report, Some(request_details.clone()))
                                                            .await;
                                                        if let Err(e) = res {
                                                            tracing::error!(
                                                                "Error adding report: {}",
                                                                e
                                                            );
                                                        }
                                                    });

                                                    response
                                                })
                                                .boxed();

                                            res
                                        }))
                                    }
                                }
                            }
                        },
                    )
                    .service(service)
                    .boxed()
            }
        }
    }
}

impl Drop for UsagePlugin {
    fn drop(&mut self) {
        self.cancellation_token.cancel();
        // Flush already done by UsageAgent's Drop impl
    }
}


#[cfg(test)]
mod hive_usage_tests {
    use apollo_router::{
        plugin::{Plugin, PluginInit, test::MockSupergraphService},
        services::supergraph,
    };
    use http::header::{AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
    use httpmock::{Method::POST, Mock, MockServer};
    use jsonschema::Validator;
    use serde_json::json;
    use tower::ServiceExt;

    use crate::consts::PLUGIN_VERSION;
    use hive_console_sdk::agent::config::{
        AtLeastOnceKey, AtLeastOnceKeyConstant, SamplerConfig, UsageReportingExclude,
    };

    use super::{Config, UsagePlugin};

    #[test]
    fn config_exclude_supports_expression_object() {
        let config: Config = serde_json::from_value(json!({
            "exclude": { "expression": ".request.operation.name == \"ExcludedOp\"" }
        }))
        .expect("config with expression object should deserialize");

        assert!(matches!(
            config.reporting.exclude,
            Some(UsageReportingExclude::Expression { .. })
        ));

        if let Some(UsageReportingExclude::Expression { expression }) = config.reporting.exclude {
            assert_eq!(
                expression,
                ".request.operation.name == \"ExcludedOp\"",
                "expression should match the input"
            );
        } else {
            panic!("Expected an expression exclude");
        }
    }

    #[test]
    fn config_exclude_supports_legacy_operation_list() {
        let config: Config = serde_json::from_value(json!({
            "exclude": ["ExcludedOp", "IntrospectionQuery"]
        }))
        .expect("config with legacy operation list should deserialize");

        assert!(matches!(
            config.reporting.exclude,
            Some(UsageReportingExclude::OperationNames(_))
        ));

        if let Some(UsageReportingExclude::OperationNames(names)) = config.reporting.exclude {
            assert_eq!(
                names,
                vec!["ExcludedOp".to_string(), "IntrospectionQuery".to_string()],
                "operation names should match the input"
            );
        } else {
            panic!("Expected an operation names exclude");
        }
    }

    #[test]
    fn config_sampler_fixed() {
        let config: Config = serde_json::from_value(json!({
            "sampler": { "type": "fixed", "rate": "25%" }
        }))
        .expect("fixed sampler config should deserialize");

        match config.reporting.sampler {
            SamplerConfig::Fixed { rate } => assert_eq!(rate.as_f64(), 0.25),
            other => panic!("expected Fixed sampler, got: {:?}", other),
        }
    }

    #[test]
    fn config_sampler_at_least_once_with_defaults() {
        let config: Config = serde_json::from_value(json!({
            "sampler": { "type": "at_least_once" }
        }))
        .expect("at_least_once with defaults should deserialize");

        match config.reporting.sampler {
            SamplerConfig::AtLeastOnce { key, rate, .. } => {
                assert!(matches!(
                    key,
                    AtLeastOnceKey::Constant(AtLeastOnceKeyConstant::OperationName)
                ));
                assert_eq!(rate.as_f64(), 0.0);
            }
            other => panic!("expected AtLeastOnce sampler, got: {:?}", other),
        }
    }

    #[test]
    fn config_sampler_at_least_once_with_key_expression() {
        let config: Config = serde_json::from_value(json!({
            "sampler": {
                "type": "at_least_once",
                "key": { "expression": ".request.headers.\"x-tenant\"" },
                "rate": "50%"
            }
        }))
        .expect("at_least_once with key expression should deserialize");

        match config.reporting.sampler {
            SamplerConfig::AtLeastOnce { key, rate, .. } => {
                assert!(
                    matches!(key, AtLeastOnceKey::Expression { ref expression } if expression == ".request.headers.\"x-tenant\"")
                );
                assert_eq!(rate.as_f64(), 0.5);
            }
            other => panic!("expected AtLeastOnce sampler, got: {:?}", other),
        }
    }

    lazy_static::lazy_static! {
        static ref SCHEMA_VALIDATOR: Validator =
                jsonschema::validator_for(&serde_json::from_str(&std::fs::read_to_string("../../../lib/hive-console-sdk/usage-report-v2.schema.json").expect("can't load json schema file")).expect("failed to parse json schema")).expect("failed to parse schema");
    }

    struct UsageTestHelper {
        mocked_upstream: MockServer,
        plugin: UsagePlugin,
    }

    impl UsageTestHelper {
        async fn new() -> Self {
            let server: MockServer = MockServer::start();
            let usage_endpoint = server.url("/usage");
            let mut config = Config::default();
            config.reporting.enabled = true;
            config.reporting.endpoint = usage_endpoint.to_string();
            config.token = Some("123".into());
            config.reporting.buffer_size = 1;
            config.reporting.flush_interval = std::time::Duration::from_secs(1);

            let plugin_service = UsagePlugin::new(
                PluginInit::fake_builder()
                    .config(config)
                    .supergraph_sdl("type Query { dummy: String! }".to_string().into())
                    .build(),
            )
            .await
            .expect("failed to init plugin");

            UsageTestHelper {
                mocked_upstream: server,
                plugin: plugin_service,
            }
        }

        fn wait_for_processing(&self) -> tokio::time::Sleep {
            tokio::time::sleep(tokio::time::Duration::from_secs(2))
        }

        fn activate_usage_mock(&'_ self) -> Mock<'_> {
            self.mocked_upstream.mock(|when, then| {
                when.method(POST)
                    .path("/usage")
                    .header(CONTENT_TYPE.as_str(), "application/json")
                    .header(
                        USER_AGENT.as_str(),
                        format!("hive-apollo-router/{}", PLUGIN_VERSION),
                    )
                    .header(AUTHORIZATION.as_str(), "Bearer 123")
                    .header("X-Usage-API-Version", "2")
                    .matches(|r| {
                        // This mock also validates that the content of the reported usage is valid
                        // when it comes to the JSON schema validation.
                        // if it does not match, the request matching will fail and this will lead
                        // to a failed assertion
                        let body = r.body.as_ref().unwrap();
                        let body = String::from_utf8(body.to_vec()).unwrap();
                        let body = serde_json::from_str(&body).unwrap();

                        SCHEMA_VALIDATOR.is_valid(&body)
                    });
                then.status(200);
            })
        }

        async fn execute_operation(&self, req: supergraph::Request) -> supergraph::Response {
            let mut supergraph_service_mock = MockSupergraphService::new();

            supergraph_service_mock
                .expect_call()
                .times(1)
                .returning(move |_| {
                    Ok(supergraph::Response::fake_builder()
                        .data(json!({
                            "data": { "hello": "world" },
                        }))
                        .build()
                        .unwrap())
                });

            let tower_service = self
                .plugin
                .supergraph_service(supergraph_service_mock.boxed());

            let response = tower_service
                .oneshot(req)
                .await
                .expect("failed to execute operation");

            response
        }
    }

    #[tokio::test]
    async fn should_work_correctly_for_simple_query() {
        let instance = UsageTestHelper::new().await;
        let req = supergraph::Request::fake_builder()
            .query("query test { hello }")
            .operation_name("test")
            .build()
            .unwrap();
        let mock = instance.activate_usage_mock();

        instance.execute_operation(req).await.next_response().await;

        instance.wait_for_processing().await;

        mock.assert();
        mock.assert_hits(1);
    }

    #[tokio::test]
    async fn without_operation_name() {
        let instance = UsageTestHelper::new().await;
        let req = supergraph::Request::fake_builder()
            .query("query { hello }")
            .build()
            .unwrap();
        let mock = instance.activate_usage_mock();

        instance.execute_operation(req).await.next_response().await;

        instance.wait_for_processing().await;

        mock.assert();
        mock.assert_hits(1);
    }

    #[tokio::test]
    async fn multiple_operations() {
        let instance = UsageTestHelper::new().await;
        let req = supergraph::Request::fake_builder()
            .query("query test { hello } query test2 { hello }")
            .operation_name("test")
            .build()
            .unwrap();
        let mock = instance.activate_usage_mock();

        instance.execute_operation(req).await.next_response().await;

        instance.wait_for_processing().await;
        println!("Waiting done");

        mock.assert();
        mock.assert_hits(1);
    }
}
