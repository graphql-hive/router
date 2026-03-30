use futures::Stream;
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
        plan::QueryPlanExecutionResult,
    },
    hooks::{on_graphql_params::GraphQLParams, on_supergraph_load::SupergraphData},
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
        header::{RequestAccepts, ResponseMode, StreamContentType, TEXT_HTML_MIME},
        introspection_policy::handle_introspection_policy,
        multipart_subscribe::{
            APOLLO_MULTIPART_HTTP_CONTENT_TYPE, INCREMENTAL_DELIVERY_CONTENT_TYPE,
        },
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
    shared_state::{RouterRequestDedupeHeaderPolicy, RouterSharedState, SharedRouterResponse},
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
pub mod http_callback;
pub mod introspection_policy;
pub mod multipart_subscribe;
pub mod normalize;
pub mod parser;
pub mod progressive_override;
pub mod query_plan;
pub mod request_extensions;
pub mod sse;
pub mod timeout;
pub mod usage_reporting;
pub mod validation;
pub mod websocket_server;

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
    let span_clone = operation_span.clone();

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
            && (!shared_state.router_config.subscriptions.enabled || !response_mode.can_stream())
        {
            // check early, even though we check again after pipeline execution below
            return Err(PipelineError::SubscriptionsNotSupported);
        }

        let request_dedupe_enabled =
            shared_state.router_config.traffic_shaping.router.dedupe.enabled;

        let planned_response = if request_dedupe_enabled
            && matches!(
                normalize_payload.operation_for_plan.operation_kind,
                Some(OperationKind::Query) | None
            ) {
            let variables_hash = hash_graphql_variables(&graphql_params.variables);
            let extensions_hash = graphql_params
                .extensions
                .as_ref()
                .map_or(0, hash_graphql_extensions);

            let schema_checksum = supergraph.schema_checksum();
            let fingerprint = inbound_request_fingerprint(
                req,
                &shared_state.in_flight_requests_header_policy,
                schema_checksum,
                normalize_payload.normalized_operation_hash,
                variables_hash,
                extensions_hash,
            );
            let (shared_response, _role) = shared_state
                .in_flight_requests
                .claim(fingerprint)
                .get_or_try_init(|| async {
                    match execute_planned_request(
                        req,
                        graphql_params,
                        &normalize_payload,
                        supergraph,
                        shared_state,
                        schema_state,
                        operation_span,
                        plugin_req_state,
                        response_mode,
                    )
                    .await?
                    {
                        PlannedResponse::Shared(r) => Ok::<SharedRouterResponse, PipelineError>(r),
                        // subscriptions are excluded from the dedup branch above, so this is unreachable
                        PlannedResponse::Direct { .. } => unreachable!("stream responses never enter the dedup path"),
                    }
                })
                .await?;

            PlannedResponse::Shared(Arc::unwrap_or_clone(shared_response))
        } else {
            execute_planned_request(
                req,
                graphql_params,
                &normalize_payload,
                supergraph,
                shared_state,
                schema_state,
                operation_span,
                plugin_req_state,
                response_mode,
            )
            .await?
        };

        let (response, error_count) = match planned_response {
            PlannedResponse::Shared(shared_response) => {
                let error_count = shared_response.error_count;
                (shared_response.into(), error_count)
            }
            PlannedResponse::Direct { response, .. } => (response, 0),
        };

        if let Some(hive_usage_agent) = &shared_state.hive_usage_agent {
            usage_reporting::collect_usage_report(
                supergraph.supergraph_schema.clone(),
                started_at.elapsed(),
                client_name,
                client_version,
                normalize_payload.operation_for_plan.name.as_deref(),
                &parser_payload.minified_document,
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
                error_count,
            )
            .await;
        }

        write_graphql_response_metric_status(
            req,
            if error_count > 0 {
                GraphQLResponseStatus::Error
            } else {
                GraphQLResponseStatus::Ok
            },
        );

        Ok(response)
    }
    .instrument(span_clone)
    .await
    .inspect_err(|_| {
        write_graphql_response_metric_status(req, GraphQLResponseStatus::Error);
    })
}

enum PlannedResponse {
    Shared(SharedRouterResponse),
    Direct { response: web::HttpResponse },
}

#[allow(clippy::too_many_arguments)]
async fn execute_planned_request<'exec>(
    req: &'exec HttpRequest,
    mut graphql_params: GraphQLParams,
    normalize_payload: &Arc<GraphQLNormalizationPayload>,
    supergraph: &'exec SupergraphData,
    shared_state: &'exec Arc<RouterSharedState>,
    schema_state: &'exec Arc<SchemaState>,
    operation_span: GraphQLOperationSpan,
    plugin_req_state: Option<PluginRequestState<'exec>>,
    response_mode: &'exec ResponseMode,
) -> Result<PlannedResponse, PipelineError> {
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
    }
    .into();

    match execute_pipeline(
        &client_request_details,
        normalize_payload,
        variable_payload,
        supergraph,
        shared_state,
        schema_state,
        operation_span,
        plugin_req_state,
    )
    .await?
    {
        QueryPlanExecutionResult::Stream(result) => {
            let stream_content_type = response_mode
                .stream_content_type()
                .ok_or(PipelineError::SubscriptionsTransportNotSupported)?;

            let content_type_header = match stream_content_type {
                StreamContentType::IncrementalDelivery => {
                    http::HeaderValue::from_static(INCREMENTAL_DELIVERY_CONTENT_TYPE)
                }
                StreamContentType::SSE => http::HeaderValue::from_static("text/event-stream"),
                StreamContentType::ApolloMultipartHTTP => {
                    http::HeaderValue::from_static(APOLLO_MULTIPART_HTTP_CONTENT_TYPE)
                }
            };

            // TODO: why exactly do we need a type cast here?
            let body: std::pin::Pin<
                Box<dyn Stream<Item = Result<ntex::util::Bytes, std::io::Error>> + Send>,
            > = match stream_content_type {
                StreamContentType::IncrementalDelivery => Box::pin(
                    multipart_subscribe::create_incremental_delivery_stream(result.body),
                ),
                StreamContentType::SSE => Box::pin(sse::create_stream(
                    result.body,
                    std::time::Duration::from_secs(10),
                )),
                StreamContentType::ApolloMultipartHTTP => {
                    Box::pin(multipart_subscribe::create_apollo_multipart_http_stream(
                        result.body,
                        std::time::Duration::from_secs(10),
                    ))
                }
            };

            let mut response_builder = web::HttpResponse::Ok();

            if let Some(response_headers_aggregator) = result.response_headers_aggregator {
                response_headers_aggregator
                    .modify_client_response_headers(&mut response_builder)?;
            }

            let response = response_builder
                // .status(result.status) status codes in streaming responses should always be ok
                .header(http::header::CONTENT_TYPE, content_type_header)
                .streaming(body);

            Ok(PlannedResponse::Direct { response })
        }
        QueryPlanExecutionResult::Single(result) => {
            let single_content_type = response_mode.
                single_content_type().
                // TODO: streaming single responses
                ok_or(PipelineError::UnsupportedContentType)?;

            let error_count = result.error_count;
            let mut response_builder = web::HttpResponse::Ok();

            if let Some(response_headers_aggregator) = result.response_headers_aggregator {
                response_headers_aggregator
                    .modify_client_response_headers(&mut response_builder)?;
            }

            let body = ntex::util::Bytes::from(result.body);

            let response = response_builder
                .content_type(single_content_type.as_ref())
                .status(result.status_code)
                .body(body.clone());

            Ok(PlannedResponse::Shared(SharedRouterResponse {
                body,
                headers: Arc::new(response.headers().clone()),
                status: response.status(),
                error_count,
            }))
        }
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub async fn execute_pipeline<'exec>(
    client_request_details: &Arc<ClientRequestDetails<'exec>>,
    normalize_payload: &Arc<GraphQLNormalizationPayload>,
    variable_payload: CoerceVariablesPayload,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
    operation_span: GraphQLOperationSpan,
    plugin_req_state: Option<PluginRequestState<'exec>>,
) -> Result<QueryPlanExecutionResult, PipelineError> {
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
        &variable_payload,
        &client_request_details.jwt,
    )?;

    let query_plan_result = plan_operation_with_cache(
        supergraph,
        schema_state,
        &normalize_payload,
        &progressive_override_ctx,
        &cancellation_token,
        &plugin_req_state,
    )
    .await?;

    let query_plan_payload = match query_plan_result {
        QueryPlanResult::QueryPlan(plan) => plan,
        QueryPlanResult::EarlyResponse(response) => {
            return Ok(QueryPlanExecutionResult::Single(response));
        }
    };

    let planned_request = PlannedRequest {
        normalized_payload: normalize_payload,
        query_plan_payload: &query_plan_payload,
        variable_payload,
        client_request_details: client_request_details.clone(),
        authorization_errors,
        plugin_req_state,
    };

    execute_plan(supergraph, shared_state, planned_request, operation_span).await
}

fn inbound_request_fingerprint(
    req: &HttpRequest,
    dedupe_header_policy: &RouterRequestDedupeHeaderPolicy,
    schema_checksum: u64,
    normalized_operation_hash: u64,
    variables_hash: u64,
    extensions_hash: u64,
) -> u64 {
    let mut hasher = Xxh3::new();

    let mut headers: Vec<(&str, &str)> = req
        .headers()
        .iter()
        .filter(|(name, _)| dedupe_header_policy.should_include(name.as_str()))
        .filter_map(|(name, value)| value.to_str().ok().map(|v_str| (name.as_str(), v_str)))
        .collect();
    headers.sort_unstable_by(|(left_name, left_value), (right_name, right_value)| {
        left_name
            .cmp(right_name)
            .then_with(|| left_value.cmp(right_value))
    });

    req.method().hash(&mut hasher);
    req.path().hash(&mut hasher);
    headers.hash(&mut hasher);
    schema_checksum.hash(&mut hasher);
    normalized_operation_hash.hash(&mut hasher);
    variables_hash.hash(&mut hasher);
    extensions_hash.hash(&mut hasher);

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

fn hash_graphql_extensions(extensions: &HashMap<String, Value>) -> u64 {
    // reused as hash_graphql_variables has the same function signature
    hash_graphql_variables(extensions)
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
