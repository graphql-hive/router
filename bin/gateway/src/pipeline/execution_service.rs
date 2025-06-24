use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::pipeline::coerce_variables_service::CoerceVariablesPayload;
use crate::pipeline::http_request_params::HttpRequestParams;
use crate::pipeline::normalize_service::GraphQLNormalizationPayload;
use crate::pipeline::query_plan_service::QueryPlanPayload;
use crate::shared_state::GatewaySharedState;
use axum::body::Body;
use axum::response::IntoResponse;
use axum::Json;
use http::header::CONTENT_TYPE;
use http::{Request, Response};
use query_plan_executor::execute_query_plan;
use serde_json::{to_value, Map};
use tower::Service;

#[derive(Clone, Debug, Default)]
pub struct ExecutionService {
    expose_query_plan: bool,
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
    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let expose_query_plan = self.expose_query_plan;
        Box::pin(async move {
            let normalized_payload = req
                .extensions()
                .get::<GraphQLNormalizationPayload>()
                .expect("GraphQLNormalizationPayload missing");
            let query_plan_payload = req
                .extensions()
                .get::<QueryPlanPayload>()
                .expect("QueryPlanPayload missing");
            let app_state = req
                .extensions()
                .get::<Arc<GatewaySharedState>>()
                .expect("GatewaySharedState is missing");

            let variable_payload = req
                .extensions()
                .get::<CoerceVariablesPayload>()
                .expect("CoerceVariablesPayload missing");

            let http_request_params = req
                .extensions()
                .get::<HttpRequestParams>()
                .expect("HttpRequestParams missing");

            let mut execution_result = execute_query_plan(
                &query_plan_payload.query_plan,
                &app_state.subgraph_executor_map,
                &variable_payload.variables_map,
                &app_state.schema_metadata,
                &normalized_payload.normalized_document.operation,
                normalized_payload.has_introspection,
            )
            .await;

            if expose_query_plan {
                let plan_value = to_value(query_plan_payload.query_plan.as_ref()).unwrap();

                execution_result
                    .extensions
                    .get_or_insert_with(Map::new)
                    .insert("queryPlan".to_string(), plan_value);
            }

            let mut response = Json(execution_result).into_response();
            response.headers_mut().insert(
                CONTENT_TYPE,
                http_request_params.response_content_type.clone(),
            );

            Ok(response)
        })
    }
}
