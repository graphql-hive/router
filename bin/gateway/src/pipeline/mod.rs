use std::sync::Arc;

use http::{HeaderValue, Method};
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};
use query_plan_executor::execute_query_plan;

use crate::{
    pipeline::{
        coerce_variables_service::coerce_vars,
        error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant},
        graphql_request_params::get_execution_request,
        header::{
            RequestAccepts, APPLICATION_GRAPHQL_RESPONSE_JSON,
            APPLICATION_GRAPHQL_RESPONSE_JSON_STR, APPLICATION_JSON,
        },
        normalize_service::normalize_op,
        parser_service::parse_operation,
        progressive_override_service::progressive_override_extractor,
        query_plan_service::plan_query,
        validation_service::validate_operation,
    },
    shared_state::GatewaySharedState,
};

pub mod coerce_variables_service;
pub mod error;
pub mod graphql_request_params;
pub mod header;
pub mod normalize_service;
pub mod parser_service;
pub mod progressive_override_service;
pub mod query_plan_service;
pub mod validation_service;

static GRAPHIQL_HTML: &str = include_str!("../../static/graphiql.html");

pub async fn graphql_request_handler(
    req: HttpRequest,
    body_bytes: Bytes,
    state: web::types::State<Arc<GatewaySharedState>>,
) -> impl web::Responder {
    if req.method() == Method::GET && req.accepts_content_type("text/html") {
        return web::HttpResponse::Ok()
            .header("Content-Type", "text/html")
            .body(GRAPHIQL_HTML);
    }

    let response = match execute_pipeline(&req, &body_bytes, &state).await {
        Ok(response_bytes) => response_bytes,
        Err(err) => {
            return err.into_response();
        }
    };

    let response_content_type: &'static HeaderValue =
        if req.accepts_content_type(*APPLICATION_GRAPHQL_RESPONSE_JSON_STR) {
            &APPLICATION_GRAPHQL_RESPONSE_JSON
        } else {
            &APPLICATION_JSON
        };

    web::HttpResponse::Ok()
        .header(http::header::CONTENT_TYPE, response_content_type)
        .body(response)
}

pub async fn execute_pipeline(
    req: &HttpRequest,
    body_bytes: &Bytes,
    state: &web::types::State<Arc<GatewaySharedState>>,
) -> Result<Vec<u8>, PipelineError> {
    let execution_request = get_execution_request(req, body_bytes)?;
    let parser_payload = parse_operation(req, &execution_request, state).await?;
    validate_operation(req, state, &parser_payload).await?;

    let progressive_override_ctx = progressive_override_extractor()?;
    let normalize_payload = normalize_op(req, &execution_request, &parser_payload, state).await?;
    let variable_payload = coerce_vars(req, &execution_request, state, &normalize_payload)?;
    let query_plan_payload =
        plan_query(req, state, &progressive_override_ctx, &normalize_payload).await?;

    let execution_result = execute_query_plan(
        &query_plan_payload.query_plan,
        &state.subgraph_executor_map,
        &variable_payload.variables_map,
        &state.schema_metadata,
        normalize_payload.root_type_name,
        &normalize_payload.projection_plan,
        normalize_payload.has_introspection,
        query_plan_executor::ExposeQueryPlanMode::No,
    )
    .await
    .map_err(|err| req.new_pipeline_error(PipelineErrorVariant::ResponseWriteError(err)))?;

    Ok(execution_result)
}
