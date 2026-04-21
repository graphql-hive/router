use futures::StreamExt;
use hive_router_internal::{
    http::read_body_stream,
    telemetry::traces::spans::{
        graphql::GraphQLOperationSpan, http_request::HttpServerRequestSpan,
    },
};
use hive_router_plan_executor::{
    coprocessor::runtime::MutableRequestState,
    execution::{
        client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
        plan::{PlanExecutionOutput, QueryPlanExecutionResult},
    },
    headers::response::ResponseHeaderAggregator,
    hooks::{on_graphql_params::GraphQLParams, on_supergraph_load::SupergraphData},
    plugin_context::{PluginContext, PluginRequestState},
    request_context::{RequestContextExt, SharedRequestContext},
};
use hive_router_query_planner::{
    state::supergraph_state::OperationKind, utils::cancellation::CancellationToken,
};
use http::{header::CONTENT_TYPE, Method};
use ntex::{
    http::body::{Body, ResponseBody},
    http::HeaderMap,
    rt,
    web::{self, HttpRequest},
};
use sonic_rs::{JsonContainerTrait, JsonType, JsonValueTrait, Value};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    ops::{ControlFlow, Deref},
    sync::Arc,
    time::Instant,
};
use tracing::{error, Instrument};
use xxhash_rust::xxh3::Xxh3;

use crate::{
    pipeline::{
        active_subscriptions::SubscriptionEvent,
        authorization::enforce_operation_authorization,
        coerce_variables::{coerce_request_variables, CoerceVariablesPayload},
        csrf_prevention::perform_csrf_prevention,
        error::PipelineError,
        execution::{execute_plan, PlannedRequest},
        execution_request::{GetQueryStr, OperationPreparation, OperationPreparationResult},
        header::{RequestAccepts, ResponseMode, TEXT_HTML_MIME},
        introspection_policy::handle_introspection_policy,
        normalize::{normalize_request_with_cache, GraphQLNormalizationPayload},
        parser::{parse_operation_with_cache, ParseResult},
        progressive_override::request_override_context,
        query_plan::{plan_operation_with_cache, QueryPlanResult},
        request_extensions::{
            write_graphql_operation_metric_identity, write_graphql_response_metric_status,
        },
        validation::validate_operation_with_cache,
    },
    schema_state::SchemaState,
    shared_state::{
        RouterRequestDedupeHeaderPolicy, RouterSharedState, SharedRouterResponse,
        SharedRouterResponseGuard, SharedRouterSingleResponse, SharedRouterStreamResponse,
    },
    LABORATORY_HTML,
};

use hive_router_internal::telemetry::metrics::catalog::values::GraphQLResponseStatus;

pub mod active_subscriptions;
pub mod authorization;
pub mod coerce_variables;
pub mod cors;
pub mod csrf_prevention;
pub mod error;
pub mod execution;
pub mod execution_request;
pub mod header;
pub mod http_callback;
pub mod introspection_policy;
pub mod long_lived_client_limit;
pub mod multipart_subscribe;
pub mod normalize;
pub mod parser;
pub mod persisted_documents;
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
    req: &mut HttpRequest,
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

    if *response_mode == ResponseMode::Laboratory {
        if shared_state.router_config.laboratory.enabled {
            return Ok(web::HttpResponse::Ok()
                .header(CONTENT_TYPE, TEXT_HTML_MIME)
                .body(LABORATORY_HTML));
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

        http_server_request_span.record_body_size(body_bytes.len());

        let mut request_headers = req.headers().clone();

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

        let mut plugin_req_state = None;

        if let (Some(plugins), Some(plugin_context)) = (
            shared_state.plugins.as_ref(),
            req.extensions().get::<Arc<PluginContext>>(),
        ) {
            plugin_req_state = Some(PluginRequestState {
                plugins: plugins.clone(),
                router_http_request: req.deref().into(),
                context: plugin_context.clone(),
            });
        }

        let operation_preparation_result = OperationPreparation::prepare(
            req,
            shared_state,
            &plugin_req_state,
            body_bytes,
            client_name,
            client_version,
        )
        .await?;

        let prepared_operation = match operation_preparation_result {
            OperationPreparationResult::Operation(prepared_operation) => prepared_operation,
            OperationPreparationResult::EarlyResponse(response) => {
                return Ok(response);
            }
        };

        let mut graphql_params = prepared_operation.graphql_params;

        write_graphql_operation_metric_identity(req, graphql_params.operation_name.clone(), None);

        let parser_result =
            parse_operation_with_cache(shared_state, &graphql_params, &plugin_req_state).await?;

        let mut parser_payload = match parser_result {
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

        let request_context = req.read_request_context()?;
        request_context.update(|ctx| {
          ctx.operation.update(parser_payload.operation_name.clone(), Some(parser_payload.operation_type.clone()));
        })?;

        if let Some(coprocessor_runtime) = shared_state.coprocessor.as_ref() {
            let graphql_sdl = if coprocessor_runtime.graphql_request_needs_sdl() {
                schema_state
                    .current_supergraph()
                    .as_ref()
                    .as_ref()
                    .map(|supergraph| supergraph.public_schema.sdl.clone())
            } else {
                None
            };

            let performed_mutations = match coprocessor_runtime
                .on_graphql_request(
                    req,
                    &mut request_headers,
                    &mut graphql_params,
                    graphql_sdl.as_deref(),
                )
                .await?
            {
                ControlFlow::Break(response) => return Ok(response),
                ControlFlow::Continue(performed_mutations) => performed_mutations,
            };

            if performed_mutations.body {
                let parser_result =
                    parse_operation_with_cache(shared_state, &graphql_params, &plugin_req_state)
                        .await?;

                parser_payload = match parser_result {
                    ParseResult::Payload(payload) => payload,
                    ParseResult::EarlyResponse(response) => {
                        return Ok(response);
                    }
                };
            }
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
            Some(normalize_payload.operation_indentity.operation_type.as_str()),
        );

        // Update the request context if the operation name or type has changed
        if
          parser_payload.operation_name.as_ref() != normalize_payload.operation_indentity.name.as_ref() ||
          parser_payload.operation_type != normalize_payload.operation_indentity.operation_type
        {
          request_context.update(|ctx| {
            ctx.operation.update(
              normalize_payload.operation_indentity.name.clone(),
              Some(normalize_payload.operation_indentity.operation_type.clone())
            );
          })?;
        }

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
            // check early, even though we check again planned execution below
            return Err(PipelineError::SubscriptionsNotSupported);
        }

        let request_dedupe_enabled =
            shared_state.router_config.traffic_shaping.router.dedupe.enabled;

        let fingerprint = if request_dedupe_enabled
            && matches!(
                normalize_payload.operation_for_plan.operation_kind,
                // same deduplication applies for queries and subscriptions
                None | Some(OperationKind::Query) | Some(OperationKind::Subscription)
            ) {
            let variables_hash = hash_graphql_variables(&graphql_params.variables);
            let extensions_hash = graphql_params
                .extensions
                .as_ref()
                .map_or(0, hash_graphql_extensions);

            let schema_checksum = supergraph.schema_checksum();
            Some(inbound_request_fingerprint(
                req.method(),
                req.path(),
                &request_headers,
                &shared_state.in_flight_requests_header_policy,
                schema_checksum,
                normalize_payload.normalized_operation_hash,
                variables_hash,
                extensions_hash,
            ))
        } else {
            None
        };

        let request_context = req.read_request_context()?;

        let exec = |guard| execute_planned_request(
            req.method(),
            req.uri(),
            Arc::new(request_headers),
            graphql_params,
            &normalize_payload,
            supergraph,
            shared_state,
            schema_state,
            operation_span,
            plugin_req_state,
            &request_context,
            response_mode,
            guard,
        );

        let shared_response = if let Some(fp) = fingerprint {
            let (shared_response, _role) = if is_subscription {
                shared_state
                    .in_flight_requests
                    .claim(fp)
                    .get_or_try_init_with_guard(|guard| exec(Some(guard)))
                    .await?
            } else {
                shared_state
                    .in_flight_requests
                    .claim(fp)
                    .get_or_try_init(|| exec(None))
                    .await?
            };
            Arc::unwrap_or_clone(shared_response)
        } else {
            exec(None).await?
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
                shared_response.error_count(),
            )
            .await;
        }

        write_graphql_response_metric_status(
            req,
            if shared_response.error_count() > 0 {
                GraphQLResponseStatus::Error
            } else {
                GraphQLResponseStatus::Ok
            },
        );

        shared_response.into_response(response_mode)
    }
    .instrument(span_clone)
    .await
    .inspect_err(|_| {
        write_graphql_response_metric_status(req, GraphQLResponseStatus::Error);
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_planned_request<'exec>(
    method: &'exec Method,
    url: &'exec http::Uri,
    headers: Arc<HeaderMap>,
    mut graphql_params: GraphQLParams,
    normalize_payload: &Arc<GraphQLNormalizationPayload>,
    supergraph: &'exec SupergraphData,
    shared_state: &'exec Arc<RouterSharedState>,
    schema_state: &'exec Arc<SchemaState>,
    operation_span: GraphQLOperationSpan,
    plugin_req_state: Option<PluginRequestState<'exec>>,
    request_context: &SharedRequestContext,
    response_mode: &'exec ResponseMode,
    guard: Option<SharedRouterResponseGuard>,
) -> Result<SharedRouterResponse, PipelineError> {
    let jwt_request_details = match &shared_state.jwt_auth_runtime {
        Some(jwt_auth_runtime) => match jwt_auth_runtime
            .validate_headers(headers.as_ref(), &shared_state.jwt_claims_cache)
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
        method,
        url,
        headers,
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
        jwt: jwt_request_details.into(),
    };

    match execute_pipeline(
        client_request_details,
        &graphql_params,
        normalize_payload,
        variable_payload,
        supergraph,
        shared_state,
        schema_state,
        operation_span,
        plugin_req_state,
        request_context,
    )
    .await?
    {
        QueryPlanExecutionResult::Stream(result) => {
            // we dont use the stream content type because subscriptions
            // can be deduplicated across transports - but we do store
            // the header value in the shared response because the user
            // might choose to not deduplicate across transport boundaries
            let stream_content_type = response_mode
                .stream_content_type()
                .ok_or(PipelineError::SubscriptionsTransportNotSupported)?;

            let (producer_handle, receiver) = shared_state.active_subscriptions.register(guard);

            // subscribe the sender before spawning the pump so the channel always has
            // at least one receiver - prevents events from being lost in the window
            // between spawn and the consumer calling subscribe()
            let sender = producer_handle.sender().clone();

            let mut body_stream = result.body;
            rt::spawn(async move {
                while let Some(chunk) = body_stream.next().await {
                    if !producer_handle.send(SubscriptionEvent::Raw(bytes::Bytes::from(chunk))) {
                        // all receivers gone, stop draining
                        break;
                    }
                }
                // dropping producer_handle closes the broadcast channel
            });

            let mut builder = web::HttpResponse::Ok();
            if let Some(aggregator) = result.response_headers_aggregator {
                aggregator.modify_client_response_headers(&mut builder)?;
            };
            builder.content_type(stream_content_type.as_ref());
            let headers = Arc::new(builder.finish().headers().clone());

            Ok(SharedRouterResponse::Stream(SharedRouterStreamResponse {
                body: sender,
                headers,
                error_count: result.error_count,
                receiver: Some(receiver),
            }))
        }
        QueryPlanExecutionResult::Single(result) => {
            let single_content_type = response_mode.
                single_content_type().
                // TODO: streaming single responses
                ok_or(PipelineError::UnsupportedContentType)?.
                clone();

            // drop the `guard` as soon as the response is ready

            let mut builder = web::HttpResponse::Ok();
            if let Some(aggregator) = result.response_headers_aggregator {
                aggregator.modify_client_response_headers(&mut builder)?;
            };
            builder.content_type(single_content_type.as_ref());
            let headers = Arc::new(builder.finish().headers().clone());

            Ok(SharedRouterResponse::Single(SharedRouterSingleResponse {
                body: ntex::util::Bytes::from(result.body),
                headers,
                status: result.status_code,
                error_count: result.error_count,
            }))
        }
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub async fn execute_pipeline<'exec>(
    mut client_request_details: ClientRequestDetails<'exec>,
    graphql_params: &GraphQLParams,
    normalize_payload: &Arc<GraphQLNormalizationPayload>,
    variable_payload: CoerceVariablesPayload,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
    operation_span: GraphQLOperationSpan,
    plugin_req_state: Option<PluginRequestState<'exec>>,
    request_context: &SharedRequestContext,
) -> Result<QueryPlanExecutionResult, PipelineError> {
    if normalize_payload.operation_for_introspection.is_some() {
        handle_introspection_policy(&shared_state.introspection_policy, &client_request_details)?;
    }

    let cancellation_token =
        CancellationToken::with_timeout(shared_state.router_config.query_planner.timeout);

    let progressive_override_ctx = request_override_context(
        &shared_state.override_labels_evaluator,
        &client_request_details,
    )?;

    let (normalize_payload, authorization_errors) = enforce_operation_authorization(
        &shared_state.router_config,
        normalize_payload,
        &supergraph.authorization,
        &supergraph.metadata,
        &variable_payload,
        &client_request_details.jwt,
    )?;

    if let Some(coprocessor_runtime) = shared_state.coprocessor.as_ref() {
        let graphql_sdl = if coprocessor_runtime.graphql_analysis_needs_sdl() {
            schema_state
                .current_supergraph()
                .as_ref()
                .as_ref()
                .map(|supergraph| supergraph.public_schema.sdl.clone())
        } else {
            None
        };

        match coprocessor_runtime
            .on_graphql_analysis(
                MutableRequestState {
                    method: client_request_details.method,
                    uri: client_request_details.url,
                    headers: Arc::make_mut(&mut client_request_details.headers),
                },
                graphql_params,
                request_context,
                graphql_sdl.as_deref(),
            )
            .await?
        {
            ControlFlow::Continue(_) => {}
            ControlFlow::Break(response) => {
                let body = match response.body() {
                    ResponseBody::Body(Body::Bytes(bytes)) => bytes.to_vec(),
                    _ => Vec::new(),
                };

                return Ok(QueryPlanExecutionResult::Single(PlanExecutionOutput {
                    body,
                    // It's an early return, so the headers from the coprocessor response
                    // should all be applied to the final response.
                    // No header propagation rules should be applied.
                    response_headers_aggregator: Some(
                        ResponseHeaderAggregator::from_early_response(response.headers()),
                    ),
                    error_count: 0,
                    status_code: response.status(),
                }));
            }
        }
    }

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
        client_request_details: client_request_details.into(),
        authorization_errors,
        plugin_req_state,
    };

    execute_plan(supergraph, shared_state, planned_request, operation_span).await
}

#[allow(clippy::too_many_arguments)]
pub fn inbound_request_fingerprint(
    method: &Method,
    path: &str,
    request_headers: &HeaderMap,
    dedupe_header_policy: &RouterRequestDedupeHeaderPolicy,
    schema_checksum: u64,
    normalized_operation_hash: u64,
    variables_hash: u64,
    extensions_hash: u64,
) -> u64 {
    let mut hasher = Xxh3::new();

    let mut headers: Vec<(&str, &str)> = request_headers
        .iter()
        .filter(|(name, _)| dedupe_header_policy.should_include(name.as_str()))
        .filter_map(|(name, value)| value.to_str().ok().map(|v_str| (name.as_str(), v_str)))
        .collect();
    headers.sort_unstable_by(|(left_name, left_value), (right_name, right_value)| {
        left_name
            .cmp(right_name)
            .then_with(|| left_value.cmp(right_value))
    });

    method.hash(&mut hasher);
    path.hash(&mut hasher);
    headers.hash(&mut hasher);
    schema_checksum.hash(&mut hasher);
    normalized_operation_hash.hash(&mut hasher);
    variables_hash.hash(&mut hasher);
    extensions_hash.hash(&mut hasher);

    hasher.finish()
}

pub fn hash_graphql_variables(variables: &HashMap<String, Value>) -> u64 {
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

pub fn hash_graphql_extensions(extensions: &HashMap<String, Value>) -> u64 {
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
