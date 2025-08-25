use std::sync::Arc;

use axum::{
    body::{Body, Bytes},
    response::{Html, IntoResponse},
};
use http::{HeaderValue, Method, Request, Response};

use crate::{
    pipeline::{
        coerce_variables::coerce_request_variables,
        error::PipelineError,
        execution::execute_plan,
        execution_request::get_execution_request,
        header::{
            RequestAccepts, APPLICATION_GRAPHQL_RESPONSE_JSON,
            APPLICATION_GRAPHQL_RESPONSE_JSON_STR, APPLICATION_JSON,
        },
        normalize::normalize_request_with_cache,
        parser::parse_operation_with_cache,
        progressive_override::request_override_context,
        query_plan::plan_operation_with_cache,
        validation::validate_operation_with_cache,
    },
    shared_state::GatewaySharedState,
};

pub mod coerce_variables;
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
    req: &mut Request<Body>,
    state: Arc<GatewaySharedState>,
) -> Response<Body> {
    if req.method() == Method::GET && req.accepts_content_type("text/html") {
        return Html(GRAPHIQL_HTML).into_response();
    }

    match execute_pipeline(req, state).await {
        Ok(response_bytes) => {
            let response_content_type: &'static HeaderValue =
                if req.accepts_content_type(*APPLICATION_GRAPHQL_RESPONSE_JSON_STR) {
                    &APPLICATION_GRAPHQL_RESPONSE_JSON
                } else {
                    &APPLICATION_JSON
                };

            let mut response = response_bytes.into_response();

            response
                .headers_mut()
                .insert(http::header::CONTENT_TYPE, response_content_type.clone());

            response
        }
        Err(err) => err.into_response(),
    }
}

#[inline]
pub async fn execute_pipeline(
    req: &mut Request<Body>,
    state: Arc<GatewaySharedState>,
) -> Result<Bytes, PipelineError> {
    let execution_request = get_execution_request(req).await?;
    let parser_payload = parse_operation_with_cache(req, &state, &execution_request).await?;
    validate_operation_with_cache(req, &state, &parser_payload).await?;

    let progressive_override_ctx = request_override_context()?;
    let normalize_payload =
        normalize_request_with_cache(req, &state, &execution_request, &parser_payload).await?;
    let variable_payload =
        coerce_request_variables(req, &state, &execution_request, &normalize_payload)?;
    let query_plan_payload =
        plan_operation_with_cache(req, &state, &normalize_payload, &progressive_override_ctx)
            .await?;

    let execution_result = execute_plan(
        req,
        &state,
        &normalize_payload,
        &query_plan_payload,
        &variable_payload,
    )
    .await?;

    Ok(execution_result)
}
