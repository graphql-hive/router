use std::sync::Arc;

use hive_router_plan_executor::execution::{
    client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
    plan::PlanExecutionOutput,
};
use hive_router_query_planner::{
    state::supergraph_state::OperationKind, utils::cancellation::CancellationToken,
};
use http::{header::CONTENT_TYPE, HeaderValue, Method};
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};

use crate::{
    jwt::context::JwtRequestContext,
    pipeline::{
        coerce_variables::coerce_request_variables,
        csrf_prevention::perform_csrf_prevention,
        error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant},
        execution::execute_plan,
        execution_request::get_execution_request,
        header::{
            RequestAccepts, APPLICATION_GRAPHQL_RESPONSE_JSON,
            APPLICATION_GRAPHQL_RESPONSE_JSON_STR, APPLICATION_JSON, TEXT_HTML_CONTENT_TYPE,
        },
        normalize::normalize_request_with_cache,
        parser::parse_operation_with_cache,
        progressive_override::request_override_context,
        query_plan::plan_operation_with_cache,
        validation::validate_operation_with_cache,
    },
    schema_state::{SchemaState, SupergraphData},
    shared_state::RouterSharedState,
};

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
pub mod validation;

static GRAPHIQL_HTML: &str = include_str!("../../static/graphiql.html");

#[inline]
pub async fn graphql_request_handler(
    req: &mut HttpRequest,
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

    if let Some(jwt) = &shared_state.jwt_auth_runtime {
        match jwt.validate_request(req) {
            Ok(_) => (),
            Err(err) => return err.make_response(),
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
        Err(err) => err.into_response(),
    }
}

#[inline]
#[allow(clippy::await_holding_refcell_ref)]
pub async fn execute_pipeline(
    req: &mut HttpRequest,
    body_bytes: Bytes,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
) -> Result<PlanExecutionOutput, PipelineError> {
    perform_csrf_prevention(req, &shared_state.router_config.csrf)?;

    let mut execution_request = get_execution_request(req, body_bytes).await?;
    let parser_payload = parse_operation_with_cache(req, shared_state, &execution_request).await?;
    validate_operation_with_cache(req, supergraph, schema_state, shared_state, &parser_payload)
        .await?;

    let normalize_payload = normalize_request_with_cache(
        req,
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

    let req_extensions = req.extensions();
    let jwt_context = req_extensions.get::<JwtRequestContext>();
    let jwt_request_details = match jwt_context {
        Some(jwt_context) => JwtRequestDetails::Authenticated {
            token: jwt_context.token_raw.as_str(),
            prefix: jwt_context.token_prefix.as_deref(),
            scopes: jwt_context.extract_scopes(),
            claims: &jwt_context
                .get_claims_value()
                .map_err(|e| req.new_pipeline_error(PipelineErrorVariant::JwtForwardingError(e)))?,
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

    let progressive_override_ctx = request_override_context(
        &shared_state.override_labels_evaluator,
        &client_request_details,
    )
    .map_err(|error| req.new_pipeline_error(PipelineErrorVariant::LabelEvaluationError(error)))?;

    let query_plan_payload = plan_operation_with_cache(
        req,
        supergraph,
        schema_state,
        &normalize_payload,
        &progressive_override_ctx,
        &query_plan_cancellation_token,
    )
    .await?;

    let execution_result = execute_plan(
        req,
        supergraph,
        shared_state,
        &normalize_payload,
        &query_plan_payload,
        &variable_payload,
        &client_request_details,
    )
    .await?;

    Ok(execution_result)
}
