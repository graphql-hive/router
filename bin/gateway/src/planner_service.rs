use std::sync::Arc;

use axum::{response::IntoResponse, Extension, Json};
use query_planner::utils::parsing::safe_parse_operation;
use serde::Deserialize;
use sonic_rs::json;

use crate::{pipeline::error::PipelineErrorVariant, shared_state::GatewaySharedState};

pub async fn supergraph_version_handler(
    state: Extension<Arc<GatewaySharedState>>,
) -> impl IntoResponse {
    json!({
        "version": state.supergraph_version
    })
    .to_string()
}

pub async fn supergraph_schema_handler(
    state: Extension<Arc<GatewaySharedState>>,
) -> impl IntoResponse {
    state.sdl.clone()
}

#[derive(Deserialize)]
pub struct PlannerServiceJsonInput {
    #[serde(rename = "operationName")]
    pub operation_name: String,
    pub query: String,
}

pub async fn planner_service_handler(
    state: Extension<Arc<GatewaySharedState>>,
    body: Json<PlannerServiceJsonInput>,
) -> impl IntoResponse {
    let result = {
        let parsed = safe_parse_operation(&body.query)
            .map_err(|err| PipelineErrorVariant::FailedToParseOperation(err))?;
    };

    "test"
}
