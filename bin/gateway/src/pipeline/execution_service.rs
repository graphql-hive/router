use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::pipeline::coerce_variables_service::CoerceVariablesPayload;
use crate::pipeline::error::{PipelineErrorFromAcceptHeader, PipelineErrorVariant};
use crate::pipeline::header::{
    RequestAccepts, APPLICATION_GRAPHQL_RESPONSE_JSON, APPLICATION_GRAPHQL_RESPONSE_JSON_STR,
    APPLICATION_JSON,
};
use crate::pipeline::normalize_service::GraphQLNormalizationPayload;
use crate::pipeline::query_plan_service::QueryPlanPayload;
use crate::shared_state::GatewaySharedState;
use axum::body::Body;
use axum::response::IntoResponse;
use executor::execute_query_plan;
use executor::execution::plan::ExecuteQueryPlanParams;
use executor::introspection::resolve::IntrospectionContext;
use http::{HeaderName, HeaderValue, Request, Response};
use tower::Service;

#[derive(Clone, Debug, Default)]
pub struct ExecutionService {
    expose_query_plan: bool,
}

static EXPOSE_QUERY_PLAN_HEADER: HeaderName = HeaderName::from_static("hive-expose-query-plan");

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExposeQueryPlanMode {
    Yes,
    No,
    DryRun,
}

impl ExecutionService {
    pub fn new(expose_query_plan: bool) -> Self {
        Self { expose_query_plan }
    }
}

impl Service<Request<Body>> for ExecutionService {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[tracing::instrument(level = "trace", name = "ExecutionService", skip_all)]
    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let mut expose_query_plan: ExposeQueryPlanMode = match self.expose_query_plan {
            true => ExposeQueryPlanMode::Yes,
            false => ExposeQueryPlanMode::No,
        };

        if let Some(expose_qp_header) = req.headers().get(&EXPOSE_QUERY_PLAN_HEADER) {
            let str_value = expose_qp_header.to_str().unwrap_or_default().trim();

            match str_value {
                "true" => expose_query_plan = ExposeQueryPlanMode::Yes,
                "false" => expose_query_plan = ExposeQueryPlanMode::No,
                "dry-run" => expose_query_plan = ExposeQueryPlanMode::DryRun,
                _ => {}
            }
        }

        let normalized_payload = req
            .extensions_mut()
            .remove::<Arc<GraphQLNormalizationPayload>>()
            .expect("GraphQLNormalizationPayload missing");
        let query_plan_payload = req
            .extensions_mut()
            .remove::<QueryPlanPayload>()
            .expect("QueryPlanPayload missing");
        let app_state = req
            .extensions_mut()
            .remove::<Arc<GatewaySharedState>>()
            .expect("GatewaySharedState is missing");

        let variable_payload = req
            .extensions_mut()
            .remove::<CoerceVariablesPayload>()
            .expect("CoerceVariablesPayload missing");

        let extensions = if expose_query_plan == ExposeQueryPlanMode::Yes
            || expose_query_plan == ExposeQueryPlanMode::DryRun
        {
            Some(HashMap::from_iter([(
                "queryPlan".to_string(),
                sonic_rs::to_value(&query_plan_payload.query_plan).unwrap(),
            )]))
        } else {
            None
        };

        Box::pin(async move {
            let introspection_context = IntrospectionContext {
                query: normalized_payload.operation_for_introspection.as_ref(),
                schema: &app_state.planner.consumer_schema.document,
                metadata: &app_state.schema_metadata,
            };

            match execute_query_plan(ExecuteQueryPlanParams {
                query_plan: &query_plan_payload.query_plan,
                projection_plan: &normalized_payload.projection_plan,
                variable_values: &variable_payload.variables_map,
                extensions,
                introspection_context: &introspection_context,
                operation_type_name: normalized_payload.root_type_name,
                executors: &app_state.subgraph_executor_map,
            })
            .await
            {
                Ok(execution_result) => {
                    let response_content_type: &'static HeaderValue =
                        if req.accepts_content_type(*APPLICATION_GRAPHQL_RESPONSE_JSON_STR) {
                            &APPLICATION_GRAPHQL_RESPONSE_JSON
                        } else {
                            &APPLICATION_JSON
                        };

                    let mut response = Response::new(Body::from(execution_result));
                    response
                        .headers_mut()
                        .insert(http::header::CONTENT_TYPE, response_content_type.clone());

                    Ok(response)
                }
                Err(err) => {
                    tracing::error!("Failed to execute query plan: {}", err);
                    Ok(req
                        .new_pipeline_error(PipelineErrorVariant::PlanExecutionError(err))
                        .into_response())
                }
            }
        })
    }
}
