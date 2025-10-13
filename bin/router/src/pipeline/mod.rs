use std::{sync::Arc, time::Instant};
use tracing::{error, Instrument};

use hive_router_internal::telemetry::traces::spans::{
    graphql::GraphQLOperationSpan, http_request::HttpServerRequestSpan,
};
use hive_router_plan_executor::{
    execution::{
        client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
        plan::PlanExecutionOutput,
    },
    hooks::on_supergraph_load::SupergraphData,
    plugin_context::{PluginContext, PluginRequestState},
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
        execution::{execute_plan, PlannedRequest},
        execution_request::{deserialize_graphql_params, DeserializationResult, GetQueryStr},
        header::RequestAccepts,
        introspection_policy::handle_introspection_policy,
        normalize::{normalize_request_with_cache, GraphQLNormalizationPayload},
        parser::{parse_operation_with_cache, ParseResult},
        progressive_override::request_override_context,
        query_plan::{plan_operation_with_cache, QueryPlanResult},
        validation::validate_operation_with_cache,
    },
    schema_state::SchemaState,
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
pub mod normalize;
pub mod parser;
pub mod progressive_override;
pub mod query_plan;
pub mod usage_reporting;
pub mod validation;

#[inline]
pub async fn graphql_request_handler(
    req: &HttpRequest,
    body_stream: web::types::Payload,
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

        let mut plugin_req_state = None;

        if let (Some(plugins), Some(plugin_context)) = (
            shared_state.plugins.as_ref(),
            req.extensions().get::<Arc<PluginContext>>(),
        ) {
            plugin_req_state = Some(PluginRequestState {
                plugins: plugins.clone(),
                router_http_request: req.into(),
                context: plugin_context.clone(),
            });
        }

        let deserialization_result =
            deserialize_graphql_params(req, body_bytes, &plugin_req_state).await?;

        let mut graphql_params = match deserialization_result {
            DeserializationResult::GraphQLParams(params) => params,
            DeserializationResult::EarlyResponse(response) => {
                return Ok(response);
            }
        };

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

        let parser_result =
            parse_operation_with_cache(shared_state, &graphql_params, &plugin_req_state).await?;

        let parser_payload = match parser_result {
            ParseResult::Payload(payload) => payload,
            ParseResult::EarlyResponse(response) => {
                return Ok(response);
            }
        };

        operation_span.record_details(
            &parser_payload.minified_document,
            (&parser_payload).into(),
            client_name,
            client_version,
            &parser_payload.hive_operation_hash,
        );

        if let Some(response) = validate_operation_with_cache(
            supergraph,
            schema_state,
            shared_state,
            &parser_payload,
            &plugin_req_state,
        )
        .await?
        {
            return Ok(response);
        }

        let normalize_payload = normalize_request_with_cache(
            supergraph,
            schema_state,
            &graphql_params,
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
        // coming soon
        // && !response_mode.can_stream()
        {
            return Err(PipelineError::SubscriptionsNotSupported);
        }

        let response_mode = req.get_response_mode();

        let Some(single_content_type) = response_mode.single_content_type() else {
            // streaming responses coming soon
            return Err(PipelineError::UnsupportedContentType);
        };

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

        let variable_payload = coerce_request_variables(
            supergraph,
            &mut graphql_params.variables,
            &normalize_payload,
        )?;

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
                query: graphql_params.get_query()?,
            },
            jwt: jwt_request_details,
        };

        let pipeline_result  = execute_pipeline(
            &client_request_details,
            &normalize_payload,
            &variable_payload,
            supergraph,
            shared_state,
            schema_state,
            &operation_span,
            &plugin_req_state,
        )
        .await?;

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
                        pipeline_result .error_count,
                    )
                    .await;
                }

        let mut response_builder = web::HttpResponse::Ok();

        if let Some(response_headers_aggregator) = pipeline_result .response_headers_aggregator {
            response_headers_aggregator.modify_client_response_headers(&mut response_builder)?;
        }

        Ok(response_builder
            .content_type(single_content_type.as_ref())
            .status(pipeline_result.status_code)
            .body(pipeline_result.body))
    }
    .instrument(operation_span.clone())
    .await
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub async fn execute_pipeline<'exec>(
    client_request_details: &ClientRequestDetails<'exec>,
    normalize_payload: &Arc<GraphQLNormalizationPayload>,
    variable_payload: &CoerceVariablesPayload,
    supergraph: &SupergraphData,
    shared_state: &Arc<RouterSharedState>,
    schema_state: &Arc<SchemaState>,
    operation_span: &GraphQLOperationSpan,
    plugin_req_state: &Option<PluginRequestState<'exec>>,
) -> Result<PlanExecutionOutput, PipelineError> {
    if normalize_payload.operation_for_introspection.is_some() {
        handle_introspection_policy(&shared_state.introspection_policy, client_request_details)?;
    }

    let cancellation_token =
        CancellationToken::with_timeout(shared_state.router_config.query_planner.timeout);

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

    let query_plan_result = plan_operation_with_cache(
        supergraph,
        schema_state,
        &normalize_payload,
        &progressive_override_ctx,
        &cancellation_token,
        plugin_req_state,
    )
    .await?;

    let query_plan_payload = match query_plan_result {
        QueryPlanResult::QueryPlan(plan) => plan,
        QueryPlanResult::EarlyResponse(response) => {
            return Ok(response);
        }
    };

    let planned_request = PlannedRequest {
        normalized_payload: &normalize_payload,
        query_plan_payload: &query_plan_payload,
        variable_payload,
        client_request_details,
        authorization_errors,
        plugin_req_state,
    };

    execute_plan(supergraph, shared_state, planned_request, operation_span).await
}
