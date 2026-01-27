use futures_util::Stream;
use std::{sync::Arc, time::Instant};
use tracing::error;

use hive_router_plan_executor::execution::{
    client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
    plan::QueryPlanExecutionResult,
};
use hive_router_query_planner::{
    state::supergraph_state::OperationKind, utils::cancellation::CancellationToken,
};
use http::Method;
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};

use crate::{
    pipeline::{
        authorization::{enforce_operation_authorization, AuthorizationDecision},
        coerce_variables::{coerce_request_variables, CoerceVariablesPayload},
        csrf_prevention::perform_csrf_prevention,
        error::PipelineError,
        execution::{execute_plan, ExposeQueryPlanMode, PlannedRequest, EXPOSE_QUERY_PLAN_HEADER},
        execution_request::get_execution_request_from_http_request,
        header::{ResponseMode, StreamContentType},
        introspection_policy::handle_introspection_policy,
        multipart_subscribe::{
            APOLLO_MULTIPART_HTTP_CONTENT_TYPE, INCREMENTAL_DELIVERY_CONTENT_TYPE,
        },
        normalize::{normalize_request_with_cache, GraphQLNormalizationPayload},
        parser::parse_operation_with_cache,
        progressive_override::request_override_context,
        query_plan::plan_operation_with_cache,
        validation::validate_operation_with_cache,
    },
    schema_state::{SchemaState, SupergraphData},
    shared_state::RouterSharedState,
};

pub mod authorization;
pub mod coerce_variables;
pub mod cors;
pub mod csrf_prevention;
pub mod error;
pub mod execution;
pub mod execution_request;
pub mod header;
pub mod introspection_policy;
pub mod multipart_subscribe;
pub mod normalize;
pub mod parser;
pub mod progressive_override;
pub mod query_plan;
pub mod sse;
pub mod usage_reporting;
pub mod validation;
pub mod websocket_server;

#[inline]
pub async fn graphql_request_handler(
    req: &HttpRequest,
    body_bytes: Bytes,
    response_mode: &ResponseMode,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
) -> Result<web::HttpResponse, PipelineError> {
    let started_at = Instant::now();

    perform_csrf_prevention(req, &shared_state.router_config.csrf)?;

    let mut execution_request =
        get_execution_request_from_http_request(req, body_bytes.clone()).await?;

    let parser_payload = parse_operation_with_cache(shared_state, &execution_request).await?;
    validate_operation_with_cache(supergraph, schema_state, shared_state, &parser_payload).await?;

    let normalize_payload = normalize_request_with_cache(
        supergraph,
        schema_state,
        &execution_request,
        &parser_payload,
    )
    .await?;

    if req.method() == Method::GET {
        if let Some(OperationKind::Mutation) = normalize_payload.operation_for_plan.operation_kind {
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
            query: &execution_request.query,
        },
        jwt: &jwt_request_details,
    };

    if normalize_payload.operation_for_introspection.is_some() {
        handle_introspection_policy(&shared_state.introspection_policy, &client_request_details)?;
    }

    let variable_payload = coerce_request_variables(
        supergraph,
        &mut execution_request.variables,
        &normalize_payload,
    )?;

    let query_plan_cancellation_token =
        CancellationToken::with_timeout(shared_state.router_config.query_planner.timeout);

    let mut expose_query_plan = ExposeQueryPlanMode::No;
    if shared_state.router_config.query_planner.allow_expose {
        if let Some(expose_qp_header) = req.headers().get(&EXPOSE_QUERY_PLAN_HEADER) {
            let str_value = expose_qp_header.to_str().unwrap_or_default().trim();
            match str_value {
                "true" => expose_query_plan = ExposeQueryPlanMode::Yes,
                "dry-run" => expose_query_plan = ExposeQueryPlanMode::DryRun,
                _ => {}
            }
        }
    }

    match execute_pipeline(
        &query_plan_cancellation_token,
        &client_request_details,
        &normalize_payload,
        &variable_payload,
        &expose_query_plan,
        supergraph,
        shared_state,
        schema_state,
    )
    .await?
    {
        QueryPlanExecutionResult::Stream(result) => {
            let stream_content_type = match response_mode.stream_content_type() {
                Some(content_type) => content_type,
                None => {
                    return Err(PipelineError::SubscriptionsTransportNotSupported);
                }
            };

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
            for (header_name, header_value) in result.headers {
                if let Some(header_name) = header_name {
                    response_builder.header(header_name, header_value);
                }
            }

            Ok(response_builder
                .header(http::header::CONTENT_TYPE, content_type_header)
                .streaming(body))
        }
        QueryPlanExecutionResult::Single(result) => {
            let single_content_type = match response_mode.single_content_type() {
                Some(content_type) => content_type,
                None => {
                    // TODO: streaming single responses
                    return Err(PipelineError::UnsupportedContentType);
                }
            };

            if shared_state.router_config.usage_reporting.enabled {
                if let Some(hive_usage_agent) = &shared_state.hive_usage_agent {
                    usage_reporting::collect_usage_report(
                        supergraph.supergraph_schema.clone(),
                        started_at.elapsed(),
                        req,
                        &client_request_details,
                        hive_usage_agent,
                        &shared_state.router_config.usage_reporting,
                        &result,
                    )
                    .await;
                }
            }

            let response_bytes = Bytes::from(result.body);
            let response_headers = result.headers;

            let mut response_builder = web::HttpResponse::Ok();
            for (header_name, header_value) in response_headers {
                if let Some(header_name) = header_name {
                    response_builder.header(header_name, header_value);
                }
            }

            Ok(response_builder
                .header(http::header::CONTENT_TYPE, single_content_type.as_ref())
                .body(response_bytes))
        }
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub async fn execute_pipeline<'exec>(
    cancellation_token: &CancellationToken,
    client_request_details: &ClientRequestDetails<'exec>,
    normalize_payload: &Arc<GraphQLNormalizationPayload>,
    variable_payload: &CoerceVariablesPayload,
    expose_query_plan: &ExposeQueryPlanMode,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
) -> Result<QueryPlanExecutionResult, PipelineError> {
    let progressive_override_ctx = request_override_context(
        &shared_state.override_labels_evaluator,
        client_request_details,
    )?;

    let decision = enforce_operation_authorization(
        &shared_state.router_config,
        normalize_payload,
        &supergraph.authorization,
        &supergraph.metadata,
        variable_payload,
        client_request_details.jwt,
    );

    let (normalize_payload, authorization_errors) = match decision {
        AuthorizationDecision::NoChange => (normalize_payload.clone(), vec![]),
        AuthorizationDecision::Modified {
            new_operation_definition,
            new_projection_plan,
            errors,
        } => {
            (
                Arc::new(GraphQLNormalizationPayload {
                    operation_for_plan: Arc::new(new_operation_definition),
                    // These are cheap Arc clones
                    operation_for_introspection: normalize_payload
                        .operation_for_introspection
                        .clone(),
                    root_type_name: normalize_payload.root_type_name,
                    projection_plan: Arc::new(new_projection_plan),
                }),
                errors,
            )
        }
        AuthorizationDecision::Reject { errors } => {
            return Err(PipelineError::AuthorizationFailed(errors))
        }
    };

    let query_plan_payload = plan_operation_with_cache(
        supergraph,
        schema_state,
        &normalize_payload,
        &progressive_override_ctx,
        cancellation_token,
    )
    .await?;

    let planned_request = PlannedRequest {
        normalized_payload: &normalize_payload,
        query_plan_payload: &query_plan_payload,
        variable_payload,
        client_request_details,
        authorization_errors,
    };

    let pipeline_result =
        execute_plan(supergraph, shared_state, expose_query_plan, planned_request).await?;

    Ok(pipeline_result)
}
