use std::sync::Arc;

use axum::body::Body;
use http::Request;
use query_plan_executor::variables::collect_variables;
use query_plan_executor::ExecutionRequest;
use serde_json::{Map, Value};
use tracing::{trace, warn};

use crate::pipeline::error::{PipelineError, PipelineErrorVariant};
use crate::pipeline::gateway_layer::{
    GatewayPipelineLayer, GatewayPipelineStepDecision, ProcessorLayer,
};
use crate::pipeline::http_request_params::HttpRequestParams;
use crate::pipeline::normalize_service::GraphQLNormalizationPayload;
use crate::shared_state::GatewaySharedState;

#[derive(Clone, Debug)]
pub struct CoerceVariablesPayload {
    pub variables_map: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Default)]
pub struct CoerceVariablesService;

impl CoerceVariablesService {
    pub fn new_layer() -> ProcessorLayer<Self> {
        ProcessorLayer::new(Self)
    }
}

#[async_trait::async_trait]
impl GatewayPipelineLayer for CoerceVariablesService {
    #[tracing::instrument(level = "trace", name = "CoerceVariablesService", skip_all)]
    async fn process(
        &self,
        mut req: Request<Body>,
    ) -> Result<(Request<Body>, GatewayPipelineStepDecision), PipelineError> {
        let normalized_operation = req
            .extensions()
            .get::<GraphQLNormalizationPayload>()
            .ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("GraphQLNormalizationPayload is missing")
            })?;

        let http_payload = req.extensions().get::<HttpRequestParams>().ok_or_else(|| {
            PipelineErrorVariant::InternalServiceError("HttpRequestParams is missing")
        })?;

        let execution_params = req.extensions().get::<ExecutionRequest>().ok_or_else(|| {
            PipelineErrorVariant::InternalServiceError("ExecutionRequest is missing")
        })?;

        let app_state = req
            .extensions()
            .get::<Arc<GatewaySharedState>>()
            .ok_or_else(|| {
                PipelineErrorVariant::InternalServiceError("GatewaySharedState is missing")
            })?;

        match collect_variables(
            &normalized_operation.operation_for_plan,
            &execution_params.variables,
            &app_state.schema_metadata,
        ) {
            Ok(values) => {
                trace!(
                    "sucessfully collected variables from incoming request: {:?}",
                    values
                );

                req.extensions_mut().insert(CoerceVariablesPayload {
                    variables_map: values,
                });

                Ok((req, GatewayPipelineStepDecision::Continue))
            }
            Err(err_msg) => {
                warn!(
                    "failed to collect variables from incoming request: {}",
                    err_msg
                );

                return Err(PipelineError::new_with_accept_header(
                    PipelineErrorVariant::VariablesCoercionError(err_msg),
                    http_payload.accept_header.clone(),
                ));
            }
        }
    }
}
