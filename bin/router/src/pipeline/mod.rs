use std::sync::Arc;

use hive_router_plan_executor::{
    execution::{
        client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
        plan::{PlanExecutionOutput, ResultWithRequest, WithResult},
    },
    hooks::{
        on_graphql_params::{OnGraphQLParamsEndPayload, OnGraphQLParamsStartPayload},
        on_supergraph_load::SupergraphData,
    },
    plugin_trait::ControlFlowResult,
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
        deserialize_graphql_params::{deserialize_graphql_params, GetQueryStr},
        error::PipelineErrorVariant,
        execution::execute_plan,
        header::{
            RequestAccepts, APPLICATION_GRAPHQL_RESPONSE_JSON,
            APPLICATION_GRAPHQL_RESPONSE_JSON_STR, APPLICATION_JSON, TEXT_HTML_CONTENT_TYPE,
        },
        normalize::normalize_request_with_cache,
        parser::{parse_operation_with_cache, ParseResult},
        progressive_override::request_override_context,
        query_plan::{plan_operation_with_cache, QueryPlanResult},
        validation::validate_operation_with_cache,
    },
    schema_state::SchemaState,
    shared_state::RouterSharedState,
};

pub mod coerce_variables;
pub mod cors;
pub mod csrf_prevention;
pub mod deserialize_graphql_params;
pub mod error;
pub mod execution;
pub mod header;
pub mod normalize;
pub mod parser;
pub mod progressive_override;
pub mod query_plan;
pub mod validation;

static GRAPHIQL_HTML: &str = include_str!("../../static/graphiql.html");

#[inline]
pub async fn graphql_request_handler(
    req: HttpRequest,
    body_bytes: Bytes,
    supergraph: &SupergraphData,
    shared_state: Arc<RouterSharedState>,
    schema_state: Arc<SchemaState>,
) -> Result<ResultWithRequest<web::HttpResponse>, PipelineErrorVariant> {
    if req.method() == Method::GET && req.accepts_content_type(*TEXT_HTML_CONTENT_TYPE) {
        if shared_state.router_config.graphiql.enabled {
            return Ok(req.with_result(
                web::HttpResponse::Ok()
                    .header(CONTENT_TYPE, *TEXT_HTML_CONTENT_TYPE)
                    .body(GRAPHIQL_HTML),
            ));
        } else {
            return Ok(req.with_result(web::HttpResponse::NotFound().into()));
        }
    }

    let jwt_context = if let Some(jwt) = &shared_state.jwt_auth_runtime {
        match jwt.validate_request(&req) {
            Ok(jwt_context) => jwt_context,
            Err(err) => return Ok(req.with_result(err.make_response())),
        }
    } else {
        None
    };

    let response_content_type: &'static HeaderValue =
        if req.accepts_content_type(*APPLICATION_GRAPHQL_RESPONSE_JSON_STR) {
            &APPLICATION_GRAPHQL_RESPONSE_JSON
        } else {
            &APPLICATION_JSON
        };

    let execution_result_with_req = execute_pipeline(
        req,
        body_bytes,
        supergraph,
        shared_state,
        schema_state,
        jwt_context,
    )
    .await?;
    let response = execution_result_with_req.result;
    let response_bytes = Bytes::from(response.body);
    let response_headers = response.headers;

    let mut response_builder = web::HttpResponse::Ok();
    for (header_name, header_value) in response_headers {
        if let Some(header_name) = header_name {
            response_builder.header(header_name, header_value);
        }
    }

    Ok(execution_result_with_req.request.with_result(
        response_builder
            .header(http::header::CONTENT_TYPE, response_content_type)
            .body(response_bytes),
    ))
}

#[inline]
#[allow(clippy::await_holding_refcell_ref)]
pub async fn execute_pipeline(
    req: HttpRequest,
    body: Bytes,
    supergraph: &SupergraphData,
    shared_state: Arc<RouterSharedState>,
    schema_state: Arc<SchemaState>,
    jwt_context: Option<JwtRequestContext>,
) -> Result<ResultWithRequest<PlanExecutionOutput>, PipelineErrorVariant> {
    perform_csrf_prevention(&req, &shared_state.router_config.csrf)?;

    /* Handle on_deserialize hook in the plugins - START */
    let mut deserialization_end_callbacks = vec![];
    let mut deserialization_payload: OnGraphQLParamsStartPayload = OnGraphQLParamsStartPayload {
        router_http_request: req,
        body,
        graphql_params: None,
    };
    for plugin in shared_state.plugins.as_ref() {
        let result = plugin.on_graphql_params(deserialization_payload);
        deserialization_payload = result.payload;
        match result.control_flow {
            ControlFlowResult::Continue => { /* continue to next plugin */ }
            ControlFlowResult::EndResponse(response) => {
                return Ok(deserialization_payload
                    .router_http_request
                    .with_result(response));
            }
            ControlFlowResult::OnEnd(callback) => {
                deserialization_end_callbacks.push(callback);
            }
        }
    }
    let graphql_params = deserialization_payload.graphql_params.unwrap_or_else(|| {
        deserialize_graphql_params(
            &deserialization_payload.router_http_request,
            deserialization_payload.body,
        )
        .expect("Failed to parse execution request")
    });

    let mut payload = OnGraphQLParamsEndPayload { graphql_params };
    for deserialization_end_callback in deserialization_end_callbacks {
        let result = deserialization_end_callback(payload);
        payload = result.payload;
        match result.control_flow {
            ControlFlowResult::Continue => { /* continue to next plugin */ }
            ControlFlowResult::EndResponse(response) => {
                return Ok(deserialization_payload
                    .router_http_request
                    .with_result(response));
            }
            ControlFlowResult::OnEnd(_) => {
                // on_end callbacks should not return OnEnd again
                unreachable!("on_end callback returned OnEnd again");
            }
        }
    }
    let mut graphql_params = payload.graphql_params;
    /* Handle on_deserialize hook in the plugins - END */

    let req = deserialization_payload.router_http_request;
    let parser_result =
        parse_operation_with_cache(req, shared_state.clone(), &graphql_params).await?;

    let mut req = parser_result.request;

    let parser_payload = match parser_result.result {
        ParseResult::Payload(payload) => payload,
        ParseResult::Response(response) => {
            return Ok(req.with_result(response));
        }
    };

    validate_operation_with_cache(
        &mut req,
        supergraph,
        schema_state.clone(),
        shared_state.clone(),
        &parser_payload,
    )
    .await?;

    let normalize_payload = normalize_request_with_cache(
        supergraph,
        schema_state.clone(),
        &graphql_params,
        &parser_payload,
    )
    .await?;

    let variable_payload =
        coerce_request_variables(&req, supergraph, &mut graphql_params, &normalize_payload)?;

    let query_plan_cancellation_token =
        CancellationToken::with_timeout(shared_state.router_config.query_planner.timeout);

    let jwt_request_details = match jwt_context {
        Some(jwt_context) => JwtRequestDetails::Authenticated {
            scopes: jwt_context.extract_scopes(),
            claims: jwt_context
                .get_claims_value()
                .map_err(PipelineErrorVariant::JwtForwardingError)?,
            token: jwt_context.token_raw,
            prefix: jwt_context.token_prefix,
        },
        None => JwtRequestDetails::Unauthenticated,
    };

    let client_request_details = ClientRequestDetails {
        method: req.method().clone(),
        url: req.uri().clone(),
        headers: req.headers().clone(),
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
        jwt: jwt_request_details,
    };

    let progressive_override_ctx = request_override_context(
        &shared_state.override_labels_evaluator,
        &client_request_details,
    )
    .map_err(PipelineErrorVariant::LabelEvaluationError)?;

    let query_plan_result = plan_operation_with_cache(
        req,
        supergraph,
        schema_state.clone(),
        normalize_payload.clone(),
        &progressive_override_ctx,
        &query_plan_cancellation_token,
        shared_state.clone(),
    )
    .await?;
    let req = query_plan_result.request;
    let query_plan_payload = match query_plan_result.result {
        QueryPlanResult::QueryPlan(plan) => plan,
        QueryPlanResult::Response(response) => {
            return Ok(req.with_result(response));
        }
    };

    let execution_result = execute_plan(
        req,
        supergraph,
        shared_state.clone(),
        normalize_payload.clone(),
        query_plan_payload,
        &variable_payload,
        &client_request_details,
    )
    .await?;

    Ok(execution_result)
}
