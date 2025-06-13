use axum::{extract::rejection::JsonRejection, http::StatusCode, response::IntoResponse, Json};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, Method},
    response::Response,
};
use axum_extra::extract::WithRejection;
use query_plan_executor::execute_query_plan_with_http_executor;
use query_plan_executor::{
    introspection::filter_introspection_fields_in_operation, variables::collect_variables,
    ExecutionRequest, ExecutionResult, GraphQLError,
};
use query_planner::{
    ast::normalization::normalize_operation, planner::plan_nodes::QueryPlan,
    state::supergraph_state::OperationKind, utils::parsing::safe_parse_operation,
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tracing::error;

use crate::AppState;

#[derive(Debug, Error)]
pub enum GraphQLHandlerError {
    // The `#[from]` attribute generates `From<JsonRejection> for GraphQLHandlerError`
    // implementation. See `thiserror` docs for more information
    #[error(transparent)]
    JsonExtractorRejection(#[from] JsonRejection),
}
// We implement `IntoResponse` so GraphQLHandlerError can be used as a response
impl IntoResponse for GraphQLHandlerError {
    fn into_response(self) -> axum::response::Response {
        let payload = json!({
            "message": self.to_string(),
            "origin": "with_rejection"
        });
        let code = match self {
            GraphQLHandlerError::JsonExtractorRejection(x) => match x {
                // Axum by default uses StatusCode::UNPROCESSABLE_ENTITY (422)
                // for JSON payload being different from expected (deserialization errors).
                // GraphQL Over HTTP specification requires us to return Status::BAD_REQUEST (400).
                // That's why you see this `IntoResponse` trait and axum-macros + axum-extra.
                // https://github.com/tokio-rs/axum/tree/a7d89541786167e990d095a03ac7bdba68d7a55a/examples/customize-extractor-error
                //
                // I wish it was as simple as: axum.onJsonDeserializationError(StatusCode::BAD_REQUEST),
                // but it's not, we move on.
                JsonRejection::JsonDataError(_) => StatusCode::BAD_REQUEST,
                JsonRejection::JsonSyntaxError(_) => StatusCode::BAD_REQUEST,
                JsonRejection::MissingJsonContentType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
        };
        (code, Json(payload)).into_response()
    }
}

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphQLQueryParams {
    pub query: Option<String>,
    pub operation_name: Option<String>,
    pub variables: Option<String>,
    pub extensions: Option<String>,
}

/// To be complaint with GraphQL-Over-HTTP spec,
/// we need to conditionally respond with either 200 or 400,
/// depending on the Accept header value.
fn build_accept_aware_graphql_error_response(
    message: String,
    code: &str,
    accept_header: &str,
) -> Response {
    let status_code = match accept_header.contains("application/json") {
        true => StatusCode::OK,
        false => StatusCode::BAD_REQUEST,
    };

    build_graphql_error_response(message, code, status_code)
}

fn build_graphql_error_response(message: String, code: &str, status_code: StatusCode) -> Response {
    let error_result = ExecutionResult {
        data: None,
        errors: Some(vec![GraphQLError {
            message,
            locations: None,
            path: None,
            extensions: Some(HashMap::from([(
                "code".to_string(),
                Value::String(code.to_string()),
            )])),
        }]),
        extensions: None,
    };
    (status_code, Json(error_result)).into_response()
}

fn build_validation_error_response(
    validation_errors: Arc<Vec<graphql_tools::validation::utils::ValidationError>>,
    status_code: StatusCode,
) -> Response {
    let error_result = ExecutionResult {
        data: None,
        errors: Some(
            validation_errors
                .iter()
                .map(|err| err.into())
                .collect::<Vec<_>>(),
        ),
        extensions: None,
    };
    (status_code, Json(error_result)).into_response()
}

/// Gets the Accept header from the request's headers.
/// If none, defaults to `application/json`
fn get_accept_header(headers: &HeaderMap) -> &str {
    headers
        .get("Accept")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("application/json")
}

pub async fn graphql_post_handler(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    WithRejection(Json(execution_request), _): WithRejection<
        Json<ExecutionRequest>,
        GraphQLHandlerError,
    >,
) -> Response {
    process_graphql_request(app_state, execution_request, headers, Method::POST).await
}

pub async fn graphql_get_handler(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<GraphQLQueryParams>,
) -> Response {
    let accept_header = get_accept_header(&headers);
    let query = match params.query {
        Some(q) => q,
        None => {
            return build_accept_aware_graphql_error_response(
                "Query parameter is required".to_string(),
                "BAD_REQUEST",
                accept_header,
            );
        }
    };

    let variables = match params.variables.as_deref() {
        Some(v_str) if !v_str.is_empty() => match serde_json::from_str(v_str) {
            Ok(vars) => Some(vars),
            Err(e) => {
                return build_accept_aware_graphql_error_response(
                    format!("Failed to parse variables JSON: {}", e),
                    "BAD_REQUEST",
                    accept_header,
                );
            }
        },
        _ => None,
    };

    let extensions = match params.extensions.as_deref() {
        Some(e_str) if !e_str.is_empty() => match serde_json::from_str(e_str) {
            Ok(exts) => Some(exts),
            Err(e) => {
                return build_accept_aware_graphql_error_response(
                    format!("Failed to parse extensions JSON: {}", e),
                    "BAD_REQUEST",
                    accept_header,
                );
            }
        },
        _ => None,
    };

    let execution_request = ExecutionRequest {
        query,
        operation_name: params.operation_name,
        variables,
        extensions,
    };

    process_graphql_request(app_state, execution_request, headers, Method::GET).await
}

async fn process_graphql_request(
    app_state: Arc<AppState>,
    execution_request: ExecutionRequest,
    headers: HeaderMap,
    method: Method,
) -> Response {
    let accept_header = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json");

    let response_content_type = if accept_header.contains("application/graphql-response+json") {
        "application/graphql-response+json"
    } else {
        "application/json"
    };

    let original_document = match safe_parse_operation(&execution_request.query) {
        Ok(doc) => doc,
        Err(err) => {
            return build_accept_aware_graphql_error_response(
                err.to_string(),
                "BAD_REQUEST",
                accept_header,
            );
        }
    };

    tracing::debug!(original_document = %original_document, "Original document parsed");

    let normalized_document = match normalize_operation(
        &app_state.planner.supergraph,
        &original_document,
        execution_request.operation_name.as_deref(),
    ) {
        Ok(doc) => doc,
        Err(err) => {
            error!("Normalization error {err}");

            return build_accept_aware_graphql_error_response(
                "Unable to detect operation AST".to_string(),
                accept_header,
                accept_header,
            );
        }
    };

    tracing::debug!(normalized_document = %normalized_document, "Normalized document prepared");

    let operation = normalized_document.operation;
    tracing::debug!(executable_operation = %operation, "Executable operation obtained");

    if method == Method::GET {
        if let Some(OperationKind::Mutation) = operation.operation_kind {
            let mut response = build_graphql_error_response(
                "Cannot perform mutations over GET".to_string(),
                "METHOD_NOT_ALLOWED",
                StatusCode::METHOD_NOT_ALLOWED,
            );
            response
                .headers_mut()
                .insert(axum::http::header::ALLOW, "POST".parse().unwrap());
            return response;
        }
    }

    let (has_introspection, filtered_operation_for_plan) =
        filter_introspection_fields_in_operation(&operation);

    // GraphQL Over HTTP specification requires us to return 400
    // (in case of Accept: application/graphql-response+json)
    // on variable value coerion failures.
    // That's why collection of variables is happening before validations.
    let variable_values = match collect_variables(
        &filtered_operation_for_plan,
        &execution_request.variables,
        &app_state.schema_metadata,
    ) {
        Ok(values) => values,
        Err(err_msg) => {
            return build_accept_aware_graphql_error_response(
                err_msg,
                "BAD_REQUEST",
                accept_header,
            );
        }
    };
    tracing::debug!(variables = ?variable_values, "Variables collected");

    let consumer_schema_ast = &app_state.planner.consumer_schema.document;
    let validation_cache_key = operation.hash();
    let validation_result = match app_state.validate_cache.get(&validation_cache_key).await {
        Some(cached_validation) => cached_validation,
        None => {
            let res = graphql_tools::validation::validate::validate(
                consumer_schema_ast,
                &original_document,
                &app_state.validation_plan,
            );
            let arc_res = Arc::new(res);
            app_state
                .validate_cache
                .insert(validation_cache_key, arc_res.clone())
                .await;
            arc_res
        }
    };

    if !validation_result.is_empty() {
        tracing::debug!(validation_errors = ?validation_result, "Validation failed");
        return build_validation_error_response(validation_result, StatusCode::OK);
    }
    tracing::debug!("Validation successful");

    let plan_cache_key = filtered_operation_for_plan.hash();

    let query_plan_arc = match app_state.plan_cache.get(&plan_cache_key).await {
        Some(plan) => plan,
        None => {
            let plan = if filtered_operation_for_plan.selection_set.is_empty() && has_introspection
            {
                QueryPlan {
                    kind: "QueryPlan".to_string(),
                    node: None,
                }
            } else {
                match app_state
                    .planner
                    .plan_from_normalized_operation(&filtered_operation_for_plan)
                {
                    Ok(p) => p,
                    Err(err) => {
                        return build_graphql_error_response(
                            err.to_string(),
                            "QUERY_PLAN_BUILD_FAILED",
                            StatusCode::INTERNAL_SERVER_ERROR,
                        );
                    }
                }
            };
            let arc_plan = Arc::new(plan);
            app_state
                .plan_cache
                .insert(plan_cache_key, arc_plan.clone())
                .await;
            arc_plan
        }
    };
    tracing::debug!(query_plan = ?query_plan_arc, "Query plan obtained/generated");

    let execution_result = execute_query_plan_with_http_executor(
        &query_plan_arc,
        &app_state.subgraph_endpoint_map,
        &variable_values,
        &app_state.schema_metadata,
        &operation,
        has_introspection,
        &app_state.http_client,
    )
    .await;

    tracing::debug!(execution_result = ?execution_result, "Execution result");

    let mut response = Json(execution_result).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        response_content_type.parse().unwrap(),
    );
    response
}
