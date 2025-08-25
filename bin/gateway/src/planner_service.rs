use std::{collections::HashSet, sync::Arc};

use axum::{response::IntoResponse, Extension, Json};
use graphql_tools::validation::validate::validate;
use http::{header::CONTENT_TYPE, StatusCode};
use query_planner::{
    ast::{document::NormalizedDocument, normalization::normalize_operation},
    graph::{PlannerOverrideContext, PERCENTAGE_SCALE_FACTOR},
    planner::plan_nodes::QueryPlan,
    utils::parsing::safe_parse_operation,
};
use rand::Rng;
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
    pub operation_name: Option<String>,
    pub query: String,
}

pub async fn planner_service_handler(
    state: Extension<Arc<GatewaySharedState>>,
    body: Json<PlannerServiceJsonInput>,
) -> impl IntoResponse {
    match plan(&body.0, &state).await {
        Ok((plan, normalized_document)) => (
            StatusCode::OK,
            [(CONTENT_TYPE, "application/json")],
            json!({
                "plan": plan,
                "normalizedOperation": normalized_document.operation.to_string()
            })
            .to_string(),
        ),
        Err(err) => (
            err.default_status_code(false),
            [(CONTENT_TYPE, "application/json")],
            json!({
                "error": err.graphql_error_message()
            })
            .to_string(),
        ),
    }
}

async fn plan(
    input: &PlannerServiceJsonInput,
    state: &GatewaySharedState,
) -> Result<(QueryPlan, NormalizedDocument), PipelineErrorVariant> {
    let parsed_operation =
        safe_parse_operation(&input.query).map_err(PipelineErrorVariant::FailedToParseOperation)?;
    let consumer_schema_ast = &state.planner.consumer_schema.document;
    let validation_errors = validate(
        consumer_schema_ast,
        &parsed_operation,
        &state.validation_plan,
    );

    if !validation_errors.is_empty() {
        return Err(PipelineErrorVariant::ValidationErrors(Arc::new(
            validation_errors,
        )));
    }

    let normalized_operation = normalize_operation(
        &state.planner.supergraph,
        &parsed_operation,
        input.operation_name.as_deref(),
    )
    .map_err(PipelineErrorVariant::NormalizationError)?;

    let request_override_context = PlannerOverrideContext::new(
        HashSet::new(),
        rand::rng().random_range(0..=(100 * PERCENTAGE_SCALE_FACTOR)),
    );

    let plan = state
        .planner
        .plan_from_normalized_operation(&normalized_operation.operation, request_override_context)
        .map_err(PipelineErrorVariant::PlannerError)?;

    Ok((plan, normalized_operation))
}
