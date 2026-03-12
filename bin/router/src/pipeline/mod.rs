use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
    time::Instant,
};
use tracing::{error, Instrument};
use xxhash_rust::xxh3::Xxh3;

use hive_router_internal::telemetry::traces::spans::{
    graphql::GraphQLOperationSpan, http_request::HttpServerRequestSpan,
};
use hive_router_plan_executor::{
    execution::{
        client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
        plan::PlanExecutionOutput,
    },
    hooks::on_graphql_params::GraphQLParams,
    hooks::on_supergraph_load::SupergraphData,
    plugin_context::{PluginContext, PluginRequestState},
};
use hive_router_query_planner::{
    state::supergraph_state::OperationKind, utils::cancellation::CancellationToken,
};
use http::{header::CONTENT_TYPE, Method};
use ntex::web::{self, HttpRequest};
use sonic_rs::{JsonContainerTrait, JsonType, JsonValueTrait, Value};

use crate::{
    pipeline::{
        authorization::enforce_operation_authorization,
        body_read::read_body_stream,
        coerce_variables::{coerce_request_variables, CoerceVariablesPayload},
        csrf_prevention::perform_csrf_prevention,
        error::PipelineError,
        execution::{execute_plan, PlannedRequest},
        execution_request::{deserialize_graphql_params, DeserializationResult, GetQueryStr},
        header::{RequestAccepts, ResponseMode, SingleContentType, TEXT_HTML_MIME},
        introspection_policy::handle_introspection_policy,
        normalize::{normalize_request_with_cache, GraphQLNormalizationPayload},
        parser::{parse_operation_with_cache, ParseResult},
        progressive_override::request_override_context,
        query_plan::{plan_operation_with_cache, QueryPlanResult},
        request_extensions::{
            write_graphql_operation_metric_identity, write_graphql_response_metric_status,
            write_request_body_size,
        },
        validation::validate_operation_with_cache,
    },
    schema_state::SchemaState,
    shared_state::{RouterSharedState, SharedRouterResponse},
    GRAPHIQL_HTML,
};

use hive_router_internal::telemetry::metrics::catalog::values::GraphQLResponseStatus;

pub mod authorization;
pub mod body_read;
pub mod coerce_variables;
pub mod cors;
pub mod csrf_prevention;
pub mod error;
pub mod execution;
pub mod execution_request;
pub mod header;
pub mod introspection_policy;
pub mod normalize;
pub mod parser;
pub mod progressive_override;
pub mod query_plan;
pub mod request_extensions;
pub mod timeout;
pub mod usage_reporting;
pub mod validation;

#[inline]
pub async fn graphql_request_handler(
    req: &HttpRequest,
    body_stream: web::types::Payload,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
    http_server_request_span: &HttpServerRequestSpan,
    response_mode: &mut ResponseMode,
) -> Result<web::HttpResponse, PipelineError> {
    // If an early CORS response is needed, return it immediately.
    if let Some(early_response) = shared_state
        .cors_runtime
        .as_ref()
        .and_then(|cors| cors.get_early_response(req))
    {
        return Ok(early_response);
    }

    // agree on the response content type
    *response_mode = req.negotiate()?;

    if *response_mode == ResponseMode::GraphiQL {
        if shared_state.router_config.graphiql.enabled {
            return Ok(web::HttpResponse::Ok()
                .header(CONTENT_TYPE, TEXT_HTML_MIME)
                .body(GRAPHIQL_HTML));
        } else {
            return Ok(web::HttpResponse::NotFound().into());
        }
    }

    let started_at = Instant::now();
    let operation_span = GraphQLOperationSpan::new();

    async {
        perform_csrf_prevention(req, &shared_state.router_config.csrf)?;

        let body_bytes = read_body_stream(
            req,
            body_stream,
            shared_state
                .router_config
                .limits
                .max_request_body_size
                .to_bytes() as usize,
        )
        .await?;

        write_request_body_size(req, body_bytes.len() as u64);
        http_server_request_span.record_body_size(body_bytes.len());

        let mut plugin_req_state = None;

        if let (Some(plugins), Some(plugin_context)) = (
            shared_state.plugins.as_ref(),
            req.extensions().get::<Arc<PluginContext>>(),
        ) {
            plugin_req_state = Some(PluginRequestState {
                plugins: plugins.clone(),
                router_http_request: req.into(),
                context: plugin_context.clone(),
            });
        }

        let deserialization_result =
            deserialize_graphql_params(req, body_bytes, &plugin_req_state).await?;

        let graphql_params = match deserialization_result {
            DeserializationResult::GraphQLParams(params) => params,
            DeserializationResult::EarlyResponse(response) => {
                return Ok(response);
            }
        };

        write_graphql_operation_metric_identity(req, graphql_params.operation_name.clone(), None);

        let client_name = req
            .headers()
            .get(
                &shared_state
                    .router_config
                    .telemetry
                    .client_identification
                    .name_header,
            )
            .and_then(|v| v.to_str().ok());
        let client_version = req
            .headers()
            .get(
                &shared_state
                    .router_config
                    .telemetry
                    .client_identification
                    .version_header,
            )
            .and_then(|v| v.to_str().ok());

        let parser_result =
            parse_operation_with_cache(shared_state, &graphql_params, &plugin_req_state).await?;

        let parser_payload = match parser_result {
            ParseResult::Payload(payload) => payload,
            ParseResult::EarlyResponse(response) => {
                return Ok(response);
            }
        };

        operation_span.record_details(
            &parser_payload.minified_document,
            (&parser_payload).into(),
            client_name,
            client_version,
            &parser_payload.hive_operation_hash,
        );

        let Some(ref supergraph) = **schema_state.current_supergraph() else {
            return Err(PipelineError::NoSupergraphAvailable);
        };

        if let Some(response) = validate_operation_with_cache(
            supergraph,
            schema_state,
            shared_state,
            &parser_payload,
            &plugin_req_state,
        )
        .await?
        {
            return Ok(response);
        }

        let normalize_payload = normalize_request_with_cache(
            supergraph,
            schema_state,
            &graphql_params,
            &parser_payload,
        )
        .await?;

        write_graphql_operation_metric_identity(
            req,
            normalize_payload.operation_indentity.name.clone(),
            Some(normalize_payload.operation_indentity.operation_type),
        );

        if req.method() == Method::GET {
            if let Some(OperationKind::Mutation) =
                normalize_payload.operation_for_plan.operation_kind
            {
                error!("Mutation is not allowed over GET, stopping");
                return Err(PipelineError::MutationNotAllowedOverHttpGet);
            }
        }

        let is_subscription = matches!(
            normalize_payload.operation_for_plan.operation_kind,
            Some(OperationKind::Subscription)
        );

        if is_subscription
        // coming soon
        // && !response_mode.can_stream()
        {
            return Err(PipelineError::SubscriptionsNotSupported);
        }

        let Some(single_content_type) = response_mode.single_content_type() else {
            // streaming responses coming soon
            return Err(PipelineError::UnsupportedContentType);
        };

        let shared_response = if shared_state
            .router_config
            .traffic_shaping
            .router
            .dedupe_enabled
            && matches!(
                normalize_payload.operation_for_plan.operation_kind,
                Some(OperationKind::Query)
            ) {
            let variables_hash = hash_graphql_variables(&graphql_params.variables);
            let schema_checksum = schema_state.current_schema_checksum();
            let fingerprint = inbound_request_fingerprint(
                req,
                schema_checksum,
                normalize_payload.normalized_operation_hash,
                variables_hash,
            );
            let cell = shared_state
                .router_in_flight_requests
                .entry(fingerprint)
                .or_default()
                .clone();

            cell.get_or_try_init(|| async {
                let pipeline_result = execute_planned_request(
                    req,
                    graphql_params,
                    &normalize_payload,
                    supergraph,
                    shared_state,
                    schema_state,
                    &operation_span,
                    &plugin_req_state,
                    started_at,
                    single_content_type,
                    client_name,
                    client_version,
                )
                .await;

                shared_state.router_in_flight_requests.remove(&fingerprint);
                pipeline_result
            })
            .await?
            .clone()
        } else {
            execute_planned_request(
                req,
                graphql_params,
                &normalize_payload,
                supergraph,
                shared_state,
                schema_state,
                &operation_span,
                &plugin_req_state,
                started_at,
                single_content_type,
                client_name,
                client_version,
            )
            .await?
        };

        let pipeline_error_count = shared_response.error_count;
        let response = shared_response.into();

        write_graphql_response_metric_status(
            req,
            if pipeline_error_count > 0 {
                GraphQLResponseStatus::Error
            } else {
                GraphQLResponseStatus::Ok
            },
        );

        Ok(response)
    }
    .instrument(operation_span.clone())
    .await
    .inspect_err(|_| {
        write_graphql_response_metric_status(req, GraphQLResponseStatus::Error);
    })
}

#[allow(clippy::too_many_arguments)]
async fn execute_planned_request<'exec>(
    req: &'exec HttpRequest,
    mut graphql_params: GraphQLParams,
    normalize_payload: &Arc<GraphQLNormalizationPayload>,
    supergraph: &'exec SupergraphData,
    shared_state: &'exec Arc<RouterSharedState>,
    schema_state: &'exec Arc<SchemaState>,
    operation_span: &'exec GraphQLOperationSpan,
    plugin_req_state: &'exec Option<PluginRequestState<'exec>>,
    started_at: Instant,
    single_content_type: &'exec SingleContentType,
    client_name: Option<&'exec str>,
    client_version: Option<&'exec str>,
) -> Result<SharedRouterResponse, PipelineError> {
    let jwt_request_details = match &shared_state.jwt_auth_runtime {
        Some(jwt_auth_runtime) => match jwt_auth_runtime
            .validate_headers(req.headers(), &shared_state.jwt_claims_cache)
            .await?
        {
            Some(jwt_context) => JwtRequestDetails::Authenticated {
                scopes: jwt_context.extract_scopes(),
                claims: jwt_context.get_claims_value()?,
                token: jwt_context.token_raw,
                prefix: jwt_context.token_prefix,
            },
            None => JwtRequestDetails::Unauthenticated,
        },
        None => JwtRequestDetails::Unauthenticated,
    };

    let variable_payload =
        coerce_request_variables(supergraph, &mut graphql_params.variables, normalize_payload)?;

    let client_request_details = ClientRequestDetails {
        method: req.method(),
        url: req.uri(),
        headers: req.headers(),
        operation: OperationDetails {
            name: normalize_payload.operation_for_plan.name.as_deref(),
            kind: match normalize_payload.operation_for_plan.operation_kind {
                Some(OperationKind::Query) => "query",
                Some(OperationKind::Mutation) => "mutation",
                Some(OperationKind::Subscription) => "subscription",
                None => "query",
            },
            query: graphql_params.get_query()?,
        },
        jwt: jwt_request_details,
    };

    let pipeline_result = execute_pipeline(
        &client_request_details,
        normalize_payload,
        &variable_payload,
        supergraph,
        shared_state,
        schema_state,
        operation_span,
        plugin_req_state,
    )
    .await?;

    if let Some(hive_usage_agent) = &shared_state.hive_usage_agent {
        usage_reporting::collect_usage_report(
            supergraph.supergraph_schema.clone(),
            started_at.elapsed(),
            client_name,
            client_version,
            &client_request_details,
            hive_usage_agent,
            shared_state
                .router_config
                .telemetry
                .hive
                .as_ref()
                .map(|c| &c.usage_reporting)
                .expect(
                    // SAFETY: According to `configure_app_from_config` in `bin/router/src/lib.rs`,
                    // the UsageAgent is only created when usage reporting is enabled.
                    // Thus, this expect should never panic.
                    "Expected Usage Reporting options to be present when Hive Usage Agent is initialized",
                ),
            pipeline_result.error_count,
        )
        .await;
    }

    let error_count = pipeline_result.error_count;
    let mut response_builder = web::HttpResponse::Ok();

    if let Some(response_headers_aggregator) = pipeline_result.response_headers_aggregator {
        response_headers_aggregator.modify_client_response_headers(&mut response_builder)?;
    }

    let body = ntex::util::Bytes::from(pipeline_result.body);

    let response = response_builder
        .content_type(single_content_type.as_ref())
        .status(pipeline_result.status_code)
        .body(body.clone());

    Ok(SharedRouterResponse {
        body,
        headers: Arc::new(response.headers().clone()),
        status: response.status(),
        error_count,
    })
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub async fn execute_pipeline<'exec>(
    client_request_details: &ClientRequestDetails<'exec>,
    normalize_payload: &Arc<GraphQLNormalizationPayload>,
    variable_payload: &CoerceVariablesPayload,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
    operation_span: &GraphQLOperationSpan,
    plugin_req_state: &Option<PluginRequestState<'exec>>,
) -> Result<PlanExecutionOutput, PipelineError> {
    if normalize_payload.operation_for_introspection.is_some() {
        handle_introspection_policy(&shared_state.introspection_policy, client_request_details)?;
    }

    let cancellation_token =
        CancellationToken::with_timeout(shared_state.router_config.query_planner.timeout);

    let progressive_override_ctx = request_override_context(
        &shared_state.override_labels_evaluator,
        client_request_details,
    )?;

    let (normalize_payload, authorization_errors) = enforce_operation_authorization(
        &shared_state.router_config,
        normalize_payload,
        &supergraph.authorization,
        &supergraph.metadata,
        variable_payload,
        &client_request_details.jwt,
    )?;

    let query_plan_result = plan_operation_with_cache(
        supergraph,
        schema_state,
        &normalize_payload,
        &progressive_override_ctx,
        &cancellation_token,
        plugin_req_state,
    )
    .await?;

    let query_plan_payload = match query_plan_result {
        QueryPlanResult::QueryPlan(plan) => plan,
        QueryPlanResult::EarlyResponse(response) => {
            return Ok(response);
        }
    };

    let planned_request = PlannedRequest {
        normalized_payload: &normalize_payload,
        query_plan_payload: &query_plan_payload,
        variable_payload,
        client_request_details,
        authorization_errors,
        plugin_req_state,
    };

    execute_plan(supergraph, shared_state, planned_request, operation_span).await
}

fn inbound_request_fingerprint(
    req: &HttpRequest,
    schema_checksum: u64,
    normalized_operation_hash: u64,
    variables_hash: u64,
) -> u64 {
    let mut hasher = Xxh3::new();

    let mut headers: Vec<(&str, &str)> = req
        .headers()
        .iter()
        .filter_map(|(name, value)| value.to_str().ok().map(|v_str| (name.as_str(), v_str)))
        .collect();
    headers.sort_unstable_by_key(|(k, _)| *k);

    req.method().hash(&mut hasher);
    req.path().hash(&mut hasher);
    headers.hash(&mut hasher);
    schema_checksum.hash(&mut hasher);
    normalized_operation_hash.hash(&mut hasher);
    variables_hash.hash(&mut hasher);

    hasher.finish()
}

fn hash_graphql_variables(variables: &HashMap<String, Value>) -> u64 {
    let mut hasher = Xxh3::new();

    let mut keys: Vec<&str> = variables.keys().map(String::as_str).collect();
    keys.sort_unstable();

    keys.len().hash(&mut hasher);
    for key in keys {
        key.hash(&mut hasher);
        if let Some(value) = variables.get(key) {
            hash_graphql_value(value, &mut hasher);
        }
    }

    hasher.finish()
}

fn hash_graphql_value(value: &Value, hasher: &mut Xxh3) {
    match value.get_type() {
        JsonType::Null => 0u8.hash(hasher),
        JsonType::Boolean => {
            1u8.hash(hasher);
            value.as_bool().unwrap_or(false).hash(hasher);
        }
        JsonType::Number => {
            2u8.hash(hasher);
            if let Some(number) = value.as_i64() {
                0u8.hash(hasher);
                number.hash(hasher);
            } else if let Some(number) = value.as_u64() {
                1u8.hash(hasher);
                number.hash(hasher);
            } else if let Some(number) = value.as_f64() {
                2u8.hash(hasher);
                number.to_bits().hash(hasher);
            }
        }
        JsonType::String => {
            3u8.hash(hasher);
            value.as_str().unwrap_or_default().hash(hasher);
        }
        JsonType::Object => {
            4u8.hash(hasher);
            if let Some(object) = value.as_object() {
                object.len().hash(hasher);
                for (key, nested_value) in object.iter() {
                    key.hash(hasher);
                    hash_graphql_value(nested_value, hasher);
                }
            }
        }
        JsonType::Array => {
            5u8.hash(hasher);
            if let Some(array) = value.as_array() {
                let slice = array.as_slice();
                slice.len().hash(hasher);
                for item in slice {
                    hash_graphql_value(item, hasher);
                }
            }
        }
    }
}
