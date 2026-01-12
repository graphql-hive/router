use std::{sync::Arc, time::Instant};

use hive_router_plan_executor::{
    execution::{
        client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
        plan::PlanExecutionOutput,
    },
    response::graphql_error::{GraphQLError, GraphQLErrorExtensions},
};
use hive_router_query_planner::{
    state::supergraph_state::OperationKind, utils::cancellation::CancellationToken,
};
use http::{header::CONTENT_TYPE, HeaderValue, Method};
use ntex::{
    http::ResponseBuilder,
    util::Bytes,
    web::{self, HttpRequest},
};

use crate::{
    pipeline::{
        authorization::{enforce_operation_authorization, AuthorizationDecision},
        coerce_variables::coerce_request_variables,
        csrf_prevention::perform_csrf_prevention,
        error::{FailedExecutionResult, PipelineError},
        execution::{execute_plan, PlannedRequest},
        execution_request::get_execution_request,
        header::{
            RequestAccepts, APPLICATION_GRAPHQL_RESPONSE_JSON,
            APPLICATION_GRAPHQL_RESPONSE_JSON_STR, APPLICATION_JSON, TEXT_HTML_CONTENT_TYPE,
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
    if req.method() == Method::GET && req.accepts_content_type(*TEXT_HTML_CONTENT_TYPE) {
        if shared_state.router_config.graphiql.enabled {
            return web::HttpResponse::Ok()
                .header(CONTENT_TYPE, *TEXT_HTML_CONTENT_TYPE)
                .body(GRAPHIQL_HTML);
        } else {
            return web::HttpResponse::NotFound().into();
        }
    }

    match execute_pipeline(req, body_bytes, supergraph, shared_state, schema_state).await {
        Ok(response) => {
            let response_bytes = Bytes::from(response.body);
            let response_headers = response.headers;

            let response_content_type: &'static HeaderValue =
                if req.accepts_content_type(*APPLICATION_GRAPHQL_RESPONSE_JSON_STR) {
                    &APPLICATION_GRAPHQL_RESPONSE_JSON
                } else {
                    &APPLICATION_JSON
                };

            let mut response_builder = web::HttpResponse::Ok();
            for (header_name, header_value) in response_headers {
                if let Some(header_name) = header_name {
                    response_builder.header(header_name, header_value);
                }
            }

            response_builder
                .header(http::header::CONTENT_TYPE, response_content_type)
                .body(response_bytes)
        }
        Err(error) => {
            let accept_ok = !req.accepts_content_type(&APPLICATION_GRAPHQL_RESPONSE_JSON_STR);

            let status = error.default_status_code(accept_ok);

            if let PipelineError::ValidationErrors(validation_errors) = error {
                let validation_error_result = FailedExecutionResult {
                    errors: Some(validation_errors.iter().map(|error| error.into()).collect()),
                };

                return ResponseBuilder::new(status).json(&validation_error_result);
            }

            if let PipelineError::AuthorizationFailed(authorization_errors) = error {
                let authorization_error_result = FailedExecutionResult {
                    errors: Some(
                        authorization_errors
                            .iter()
                            .map(|error| error.into())
                            .collect(),
                    ),
                };

                return ResponseBuilder::new(status).json(&authorization_error_result);
            }

            let code = error.graphql_error_code();
            let message = error.graphql_error_message();

            let graphql_error = GraphQLError::from_message_and_code(message, code);

            let result = FailedExecutionResult {
                errors: Some(vec![graphql_error]),
            };

            ResponseBuilder::new(status).json(&result)
        }
    }
}

#[inline]
#[allow(clippy::await_holding_refcell_ref)]
pub async fn execute_pipeline(
    req: &HttpRequest,
    body_bytes: Bytes,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
) -> Result<PlanExecutionOutput, PipelineError> {
    let start = Instant::now();
    perform_csrf_prevention(req, &shared_state.router_config.csrf)?;
    let jwt_request_details = match &shared_state.jwt_auth_runtime {
        Some(jwt_auth_runtime) => match jwt_auth_runtime
            .validate_request(req, &shared_state.jwt_claims_cache)
            .await
            .map_err(PipelineError::JwtError)?
        {
            Some(jwt_context) => JwtRequestDetails::Authenticated {
                scopes: jwt_context.extract_scopes(),
                claims: jwt_context
                    .get_claims_value()
                    .map_err(PipelineError::JwtForwardingError)?,
                token: jwt_context.token_raw,
                prefix: jwt_context.token_prefix,
            },
            None => JwtRequestDetails::Unauthenticated,
        },
        None => JwtRequestDetails::Unauthenticated,
    };

    let mut execution_request = get_execution_request(req, body_bytes).await?;
    let parser_payload = parse_operation_with_cache(shared_state, &execution_request).await?;
    validate_operation_with_cache(supergraph, schema_state, shared_state, &parser_payload).await?;

    let normalize_payload = normalize_request_with_cache(
        supergraph,
        schema_state,
        &execution_request,
        &parser_payload,
    )
    .await?;
    let variable_payload =
        coerce_request_variables(req, supergraph, &mut execution_request, &normalize_payload)?;

    let query_plan_cancellation_token =
        CancellationToken::with_timeout(shared_state.router_config.query_planner.timeout);

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

    let progressive_override_ctx = request_override_context(
        &shared_state.override_labels_evaluator,
        &client_request_details,
    )
    .map_err(PipelineError::LabelEvaluationError)?;

    let decision = enforce_operation_authorization(
        &shared_state.router_config,
        &normalize_payload,
        &supergraph.authorization,
        &supergraph.metadata,
        &variable_payload,
        &jwt_request_details,
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
        &query_plan_cancellation_token,
    )
    .await?;

    let planned_request = PlannedRequest {
        normalized_payload: &normalize_payload,
        query_plan_payload: &query_plan_payload,
        variable_payload: &variable_payload,
        client_request_details: &client_request_details,
        authorization_errors: &authorization_errors,
    };
    let execution_result = execute_plan(req, supergraph, shared_state, &planned_request).await?;

    if shared_state.router_config.usage_reporting.enabled {
        if let Some(hive_usage_agent) = &shared_state.hive_usage_agent {
            usage_reporting::collect_usage_report(
                supergraph.supergraph_schema.clone(),
                start.elapsed(),
                req,
                &client_request_details,
                hive_usage_agent,
                &shared_state.router_config.usage_reporting,
                &execution_result,
            )
            .await;
        }
    }

    Ok(execution_result)
}
