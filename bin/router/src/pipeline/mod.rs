use std::{sync::Arc, time::Instant};
use tracing::error;

use hive_router_plan_executor::execution::{
    client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
    plan::PlanExecutionOutput,
};
use hive_router_query_planner::{
    state::supergraph_state::OperationKind, utils::cancellation::CancellationToken,
};
use http::{header::CONTENT_TYPE, Method};
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
        header::{RequestAccepts, TEXT_HTML_CONTENT_TYPE},
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
pub mod normalize;
pub mod parser;
pub mod progressive_override;
pub mod query_plan;
pub mod usage_reporting;
pub mod validation;

static GRAPHIQL_HTML: &str = include_str!("../../static/graphiql.html");

#[inline]
pub async fn graphql_request_handler(
    req: &HttpRequest,
    body_bytes: Bytes,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
) -> web::HttpResponse {
    let (single_content_type, _stream_content_type) = match req.accepted_content_type() {
        Ok((single, stream)) => (single, stream),
        Err(err) => return err.into_response(None),
    };

    if req.method() == Method::GET
        && single_content_type.is_none()
        // coming soon
        // && stream_content_type.is_none()
        && req.can_accept_http()
    {
        if shared_state.router_config.graphiql.enabled {
            return web::HttpResponse::Ok()
                .header(CONTENT_TYPE, *TEXT_HTML_CONTENT_TYPE)
                .body(GRAPHIQL_HTML);
        } else {
            return web::HttpResponse::NotFound().into();
        }
    }

    let started_at = Instant::now();

    if let Err(err) = perform_csrf_prevention(req, &shared_state.router_config.csrf) {
        return err.into_response(single_content_type);
    }

    let mut execution_request =
        match get_execution_request_from_http_request(req, body_bytes.clone()).await {
            Ok(exec_req) => exec_req,
            Err(err) => return err.into_response(single_content_type),
        };

    let parser_payload = match parse_operation_with_cache(shared_state, &execution_request).await {
        Ok(payload) => payload,
        Err(err) => return err.into_response(single_content_type),
    };
    if let Err(err) =
        validate_operation_with_cache(supergraph, schema_state, shared_state, &parser_payload).await
    {
        return err.into_response(single_content_type);
    }

    let normalize_payload = match normalize_request_with_cache(
        supergraph,
        schema_state,
        &execution_request,
        &parser_payload,
    )
    .await
    {
        Ok(payload) => payload,
        Err(err) => return err.into_response(single_content_type),
    };
    if req.method() == Method::GET {
        if let Some(OperationKind::Mutation) = normalize_payload.operation_for_plan.operation_kind {
            error!("Mutation is not allowed over GET, stopping");
            return PipelineError::MutationNotAllowedOverHttpGet.into_response(single_content_type);
        }
    }

    let is_subscription = matches!(
        normalize_payload.operation_for_plan.operation_kind,
        Some(OperationKind::Subscription)
    );

    if is_subscription
    // coming soon
    // && stream_content_type.is_none()
    {
        return PipelineError::SubscriptionsNotSupport.into_response(single_content_type);
    }

    let variable_payload =
        match coerce_request_variables(supergraph, &mut execution_request, &normalize_payload) {
            Ok(payload) => payload,
            Err(err) => return err.into_response(single_content_type),
        };

    let query_plan_cancellation_token =
        CancellationToken::with_timeout(shared_state.router_config.query_planner.timeout);

    let jwt_request_details = match &shared_state.jwt_auth_runtime {
        Some(jwt_auth_runtime) => match jwt_auth_runtime
            .validate_headers(req.headers(), &shared_state.jwt_claims_cache)
            .await
        {
            Ok(Some(jwt_context)) => JwtRequestDetails::Authenticated {
                scopes: jwt_context.extract_scopes(),
                claims: match jwt_context.get_claims_value() {
                    Ok(claims) => claims,
                    Err(e) => {
                        return PipelineError::JwtForwardingError(e)
                            .into_response(single_content_type);
                    }
                },
                token: jwt_context.token_raw,
                prefix: jwt_context.token_prefix,
            },
            Ok(None) => JwtRequestDetails::Unauthenticated,
            Err(e) => {
                return PipelineError::JwtError(e).into_response(single_content_type);
            }
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
    .await
    {
        Ok(response) => {
            if shared_state.router_config.usage_reporting.enabled {
                if let Some(hive_usage_agent) = &shared_state.hive_usage_agent {
                    usage_reporting::collect_usage_report(
                        supergraph.supergraph_schema.clone(),
                        started_at.elapsed(),
                        req,
                        &client_request_details,
                        hive_usage_agent,
                        &shared_state.router_config.usage_reporting,
                        &response,
                    )
                    .await;
                }
            }

            let accepted_content_type = match single_content_type {
                Some(content_type) => content_type.as_str(),
                None => {
                    return PipelineError::UnsupportedContentType
                        .into_response(single_content_type);
                }
            };

            let response_bytes = Bytes::from(response.body);
            let response_headers = response.headers;

            let mut response_builder = web::HttpResponse::Ok();
            for (header_name, header_value) in response_headers {
                if let Some(header_name) = header_name {
                    response_builder.header(header_name, header_value);
                }
            }

            response_builder
                .header(http::header::CONTENT_TYPE, accepted_content_type)
                .body(response_bytes)
        }
        Err(err) => err.into_response(single_content_type),
    }
}

#[inline]
#[allow(clippy::await_holding_refcell_ref, clippy::too_many_arguments)]
pub async fn execute_pipeline<'exec, 'req>(
    cancellation_token: &CancellationToken,
    client_request_details: &ClientRequestDetails<'exec, 'req>,
    normalize_payload: &Arc<GraphQLNormalizationPayload>,
    variable_payload: &CoerceVariablesPayload,
    expose_query_plan: &ExposeQueryPlanMode,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
) -> Result<PlanExecutionOutput, PipelineError> {
    let progressive_override_ctx = request_override_context(
        &shared_state.override_labels_evaluator,
        client_request_details,
    )
    .map_err(PipelineError::LabelEvaluationError)?;

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
        authorization_errors: &authorization_errors,
    };

    let pipeline_result = execute_plan(
        supergraph,
        shared_state,
        expose_query_plan,
        &planned_request,
    )
    .await?;

    Ok(pipeline_result)
}
