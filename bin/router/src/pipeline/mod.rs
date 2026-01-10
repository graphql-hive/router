use std::{sync::Arc, time::Instant};

use futures_util::Stream;
use hive_router_plan_executor::execution::{
    client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
    plan::QueryPlanExecutionResult,
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
        authorization::{enforce_operation_authorization, AuthorizationDecision},
        coerce_variables::coerce_request_variables,
        csrf_prevention::perform_csrf_prevention,
        error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant},
        execution::{execute_plan, PlannedRequest},
        execution_request::get_execution_request,
        header::{
            RequestAccepts, APPLICATION_GRAPHQL_RESPONSE_JSON,
            APPLICATION_GRAPHQL_RESPONSE_JSON_STR, APPLICATION_JSON, MULTIPART_MIXED,
            TEXT_EVENT_STREAM, TEXT_HTML_CONTENT_TYPE,
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
pub mod multipart_subscribe;
pub mod normalize;
pub mod parser;
pub mod progressive_override;
pub mod query_plan;
pub mod sse;
pub mod usage_reporting;
pub mod validation;
pub mod websocket;

static GRAPHIQL_HTML: &str = include_str!("../../static/graphiql.html");

#[inline]
pub async fn graphql_request_handler(
    req: &mut HttpRequest,
    body_bytes: Bytes,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
) -> web::HttpResponse {
    if req.method() == Method::GET && req.accepts_content_type(*TEXT_HTML_CONTENT_TYPE, None) {
        if shared_state.router_config.graphiql.enabled {
            return web::HttpResponse::Ok()
                .header(CONTENT_TYPE, *TEXT_HTML_CONTENT_TYPE)
                .body(GRAPHIQL_HTML);
        } else {
            return web::HttpResponse::NotFound().into();
        }
    }

    if let Some(jwt) = &shared_state.jwt_auth_runtime {
        match jwt
            .validate_request(req, &shared_state.jwt_claims_cache)
            .await
        {
            Ok(_) => (),
            Err(err) => return err.make_response(),
        }
    }

    match execute_pipeline(req, body_bytes, supergraph, shared_state, schema_state).await {
        Ok(QueryPlanExecutionResult::Single(response)) => {
            let response_bytes = Bytes::from(response.body);
            let response_headers = response.headers;

            let response_content_type: &'static HeaderValue =
                if req.accepts_content_type(*APPLICATION_GRAPHQL_RESPONSE_JSON_STR, None) {
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
        Ok(QueryPlanExecutionResult::Stream(response)) => {
            use crate::pipeline::{header::TEXT_EVENT_STREAM, multipart_subscribe, sse};

            // TODO: respect order of Accept header
            #[allow(clippy::type_complexity)]
            let (response_content_type, body): (
                http::HeaderValue,
                std::pin::Pin<
                    Box<dyn Stream<Item = Result<ntex::util::Bytes, std::io::Error>> + Send>,
                >,
            ) = if req.accepts_content_type(*TEXT_EVENT_STREAM, None) {
                (
                    http::HeaderValue::from_static("text/event-stream"),
                    Box::pin(sse::create_stream(
                        response.body,
                        std::time::Duration::from_secs(10),
                    )),
                )
            } else if req.accepts_content_type(*MULTIPART_MIXED, Some(r#"subscriptionSpec="1.0""#))
            {
                (
                    http::HeaderValue::from_static("multipart/mixed;boundary=graphql"),
                    Box::pin(multipart_subscribe::create_apollo_multipart_http_stream(
                        response.body,
                        std::time::Duration::from_secs(10),
                    )),
                )
            } else {
                // NOTE: client accept headers have been validated before. it's safe to default here.
                (
                    http::HeaderValue::from_static("multipart/mixed;boundary=-"),
                    Box::pin(multipart_subscribe::create_incremental_delivery_stream(
                        response.body,
                    )),
                )
            };

            let mut response_builder = web::HttpResponse::Ok();
            for (header_name, header_value) in response.headers {
                if let Some(header_name) = header_name {
                    response_builder.header(header_name, header_value);
                }
            }

            response_builder
                .header(http::header::CONTENT_TYPE, response_content_type)
                .streaming(body)
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
) -> Result<QueryPlanExecutionResult, PipelineError> {
    let start = Instant::now();
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

    let is_subscription = matches!(
        normalize_payload.operation_for_plan.operation_kind,
        Some(OperationKind::Subscription)
    );

    if is_subscription && !shared_state.router_config.subscriptions.enabled {
        return Err(req.new_pipeline_error(PipelineErrorVariant::SubscriptionsNotSupport));
    }

    if is_subscription
        && !req.accepts_content_type(*MULTIPART_MIXED, None)
        // considers both GraphQL's Incremental Delivery RFC and Apollo's Multipart HTTP
        && !req.accepts_content_type(*TEXT_EVENT_STREAM, None)
    {
        return Err(
            req.new_pipeline_error(PipelineErrorVariant::SubscriptionsTransportNotSupported)
        );
    }

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
            return Err(
                req.new_pipeline_error(PipelineErrorVariant::AuthorizationFailed(
                    errors.iter().map(|e| e.into()).collect(),
                )),
            )
        }
    };

    let query_plan_payload = plan_operation_with_cache(
        req,
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

    let pipeline_result = execute_plan(req, supergraph, shared_state, &planned_request).await?;

    // TODO: this needs to work for subscriptions too
    if let QueryPlanExecutionResult::Single(ref execution_result) = pipeline_result {
        if shared_state.router_config.usage_reporting.enabled {
            if let Some(hive_usage_agent) = &shared_state.hive_usage_agent {
                usage_reporting::collect_usage_report(
                    supergraph.supergraph_schema.clone(),
                    start.elapsed(),
                    req,
                    &client_request_details,
                    hive_usage_agent,
                    &shared_state.router_config.usage_reporting,
                    execution_result,
                );
            }
        }
    }

    Ok(pipeline_result)
}
