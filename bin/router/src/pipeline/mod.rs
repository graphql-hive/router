use std::{borrow::Cow, sync::Arc};

use hive_router_plan_executor::execution::plan::PlanExecutionOutput;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use http::{header::CONTENT_TYPE, HeaderValue, Method};
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};

use crate::{
    pipeline::{
        coerce_variables::coerce_request_variables,
        csrf_prevention::perform_csrf_prevention,
        error::PipelineError,
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
    state: &Arc<RouterSharedState>,
) -> web::HttpResponse {
    if req.method() == Method::GET && req.accepts_content_type(*TEXT_HTML_CONTENT_TYPE) {
        return web::HttpResponse::Ok()
            .header(CONTENT_TYPE, *TEXT_HTML_CONTENT_TYPE)
            .body(GRAPHIQL_HTML);
    }

    if let Some(jwt) = &state.jwt_auth_runtime {
        match jwt.validate_request(req) {
            Ok(_) => (),
            Err(err) => return err.make_response(),
        }
    }

    match execute_pipeline(req, body_bytes, state).await {
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
pub async fn execute_pipeline(
    req: &mut HttpRequest,
    body_bytes: Bytes,
    state: &Arc<RouterSharedState>,
) -> Result<PlanExecutionOutput, PipelineError> {
    perform_csrf_prevention(req, &state.router_config.csrf)?;

    let execution_request = get_execution_request(req, body_bytes).await?;
    let parser_payload = parse_operation_with_cache(req, state, &execution_request).await?;
    validate_operation_with_cache(req, state, &parser_payload).await?;

    let progressive_override_ctx = request_override_context()?;
    let normalize_payload =
        normalize_request_with_cache(req, state, &execution_request, &parser_payload).await?;
    let query = Cow::Owned(execution_request.query.clone());
    let variable_payload =
        coerce_request_variables(req, state, execution_request, &normalize_payload)?;

    let query_plan_cancellation_token =
        CancellationToken::with_timeout(state.router_config.query_planner.timeout);

    let query_plan_payload = plan_operation_with_cache(
        req,
        state,
        &normalize_payload,
        &progressive_override_ctx,
        &query_plan_cancellation_token,
    )
    .await?;

    let execution_result = execute_plan(
        req,
        query,
        state,
        &normalize_payload,
        &query_plan_payload,
        &variable_payload,
    )
    .await?;

    Ok(execution_result)
}
