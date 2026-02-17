use futures::Stream;
use std::{sync::Arc, time::Instant};
use tracing::{error, Instrument};

use hive_router_internal::telemetry::traces::spans::{
    graphql::GraphQLOperationSpan, http_request::HttpServerRequestSpan,
};
use hive_router_plan_executor::execution::{
    client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
    plan::QueryPlanExecutionResult,
};
use hive_router_query_planner::{
    state::supergraph_state::OperationKind, utils::cancellation::CancellationToken,
};
use http::Method;
use ntex::web::{self, HttpRequest};

use crate::{
    pipeline::{
        authorization::enforce_operation_authorization,
        body_read::read_body_stream,
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
pub mod body_read;
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
    body_stream: web::types::Payload,
    response_mode: &ResponseMode,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
    http_server_request_span: &HttpServerRequestSpan,
) -> Result<web::HttpResponse, PipelineError> {
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

        http_server_request_span.record_body_size(body_bytes.len());

        let mut execution_request = get_execution_request_from_http_request(req, body_bytes).await?;

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

        let parser_payload = parse_operation_with_cache(shared_state, &execution_request).await?;
        operation_span.record_details(
            &parser_payload.minified_document,
            (&parser_payload).into(),
            client_name,
            client_version,
            &parser_payload.hive_operation_hash,
        );

        validate_operation_with_cache(supergraph, schema_state, shared_state, &parser_payload)
            .await?;

        let normalize_payload = normalize_request_with_cache(
            supergraph,
            schema_state,
            &execution_request,
            &parser_payload,
        )
        .await?;

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
            jwt: jwt_request_details,
        };

        if normalize_payload.operation_for_introspection.is_some() {
            handle_introspection_policy(
                &shared_state.introspection_policy,
                &client_request_details,
            )?;
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
            &operation_span,
        )
        .await?
        {
            QueryPlanExecutionResult::Stream(result) => {
                let stream_content_type = response_mode.
                    stream_content_type().
                    ok_or(PipelineError::SubscriptionsTransportNotSupported)?;

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
                    response_headers_aggregator.modify_client_response_headers(&mut response_builder)?;
                }

                Ok(response_builder
                    .header(http::header::CONTENT_TYPE, content_type_header)
                    .streaming(body))
            },
            QueryPlanExecutionResult::Single(result) => {
                let single_content_type = response_mode.
                    single_content_type().
                    // TODO: streaming single responses
                    ok_or(PipelineError::UnsupportedContentType)?;

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
                        result.error_count,
                    )
                    .await;
                }

                let mut response_builder = web::HttpResponse::Ok();

                if let Some(response_headers_aggregator) = result.response_headers_aggregator {
                    response_headers_aggregator.modify_client_response_headers(&mut response_builder)?;
                }

                Ok(response_builder
                    .content_type(single_content_type.as_ref())
                    .body(result.body))
            },
        }
    }
    .instrument(operation_span.clone())
    .await
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
    operation_span: &GraphQLOperationSpan,
) -> Result<QueryPlanExecutionResult, PipelineError> {
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

    let pipeline_result = execute_plan(
        supergraph,
        shared_state,
        expose_query_plan,
        planned_request,
        operation_span,
    )
    .await?;

    Ok(pipeline_result)
}
