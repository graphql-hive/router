use std::sync::Arc;

use executor::{
    execute_query_plan, execution::plan::QueryPlanExecutionContext,
    introspection::resolve::IntrospectionContext,
};
use http::{HeaderValue, Method};
use ntex::{
    util::Bytes,
    web::{self, HttpRequest},
};

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
) -> Result<Bytes, PipelineError> {
    let execution_request = get_execution_request(req, body_bytes)?;
    let parser_payload = parse_operation(req, &execution_request, state).await?;
    validate_operation(req, state, &parser_payload).await?;

    let progressive_override_ctx = progressive_override_extractor()?;
    let normalized_payload = normalize_op(req, &execution_request, &parser_payload, state).await?;
    let variable_payload = coerce_vars(req, execution_request, state, &normalized_payload)?;
    let query_plan_payload =
        plan_query(req, state, &progressive_override_ctx, &normalized_payload).await?;

    let introspection_context = IntrospectionContext {
        query: normalized_payload.operation_for_introspection.as_ref(),
        schema: &state.planner.consumer_schema.document,
        metadata: &state.schema_metadata,
    };

    let execution_result = execute_query_plan(QueryPlanExecutionContext {
        query_plan: &query_plan_payload.query_plan,
        projection_plan: &normalized_payload.projection_plan,
        variable_values: &variable_payload.variables_map,
        extensions: None,
        introspection_context: &introspection_context,
        operation_type_name: normalized_payload.root_type_name,
        executors: &state.subgraph_executor_map,
    })
    .await
    .map_err(|err| req.new_pipeline_error(PipelineErrorVariant::PlanExecutionError(err)))?;

    Ok(execution_result.into())
}
