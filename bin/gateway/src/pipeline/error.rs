use hashbrown::HashMap;
use std::sync::Arc;

use axum::{body::Body, extract::rejection::QueryRejection, response::IntoResponse};
use graphql_tools::validation::utils::ValidationError;
use http::{Response, StatusCode};
use query_plan_executor::{ExecutionResult, GraphQLError};
use query_planner::{ast::normalization::error::NormalizationError, planner::PlannerError};
use serde_json::Value;

use crate::pipeline::http_request_params::APPLICATION_JSON;

#[derive(Debug)]
pub struct PipelineError {
    pub accept_header: Option<String>,
    pub error: PipelineErrorVariant,
}

impl PipelineError {
    pub fn new_with_accept_header(
        error: PipelineErrorVariant,
        accept_header_value: String,
    ) -> Self {
        Self {
            accept_header: Some(accept_header_value),
            error,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineErrorVariant {
    // Internal errors
    #[error("Internal service error: {0}")]
    InternalServiceError(&'static str),

    // HTTP-related errors
    #[error("Unsupported HTTP method: {0}")]
    UnsupportedHttpMethod(String),
    #[error("Header '{0}' has invalid value")]
    InvalidHeaderValue(String),
    #[error("Failed to read body: {0}")]
    FailedToReadBodyBytes(axum::Error),
    #[error("Content-Type header is missing")]
    MissingContentTypeHeader,
    #[error("Content-Type header is not supported")]
    UnsupportedContentType,

    // GET Specific pipeline errors
    #[error("Failed to deserialize query parameters: {0}")]
    GetInvalidQueryParams(QueryRejection),
    #[error("Missing query parameter: {0}")]
    GetMissingQueryParam(&'static str),
    #[error("Cannot perform mutations over GET")]
    MutationNotAllowedOverHttpGet,

    // GraphQL-specific errors
    #[error("Failed to parse GraphQL request payload")]
    FailedToParseBody(sonic_rs::Error),
    #[error("Failed to parse GraphQL variables JSON")]
    FailedToParseVariables(sonic_rs::Error),
    #[error("Failed to parse GraphQL extensions JSON")]
    FailedToParseExtensions(sonic_rs::Error),
    #[error("Failed to parse GraphQL operation")]
    FailedToParseOperation(graphql_parser::query::ParseError),
    #[error("Failed to normalize GraphQL operation")]
    NormalizationError(NormalizationError),
    #[error("Failed to collect GraphQL variables: {0}")]
    VariablesCoercionError(String),
    #[error("Validation errors")]
    ValidationErrors(Arc<Vec<ValidationError>>),
    #[error("Failed to produce a plan: {0}")]
    PlannerError(PlannerError),
}

impl PipelineErrorVariant {
    pub fn graphql_error_code(&self) -> &'static str {
        match self {
            Self::UnsupportedHttpMethod(_) => "METHOD_NOT_ALLOWED",
            Self::PlannerError(_) => "QUERY_PLAN_BUILD_FAILED",
            Self::InternalServiceError(_) => "INTERNAL_SERVER_ERROR",
            Self::FailedToParseOperation(_) => "GRAPHQL_PARSE_FAILED",
            Self::ValidationErrors(_) => "GRAPHQL_VALIDATION_FAILED",
            Self::VariablesCoercionError(_) => "BAD_USER_INPUT",
            Self::NormalizationError(NormalizationError::OperationNotFound) => {
                "OPERATION_RESOLUTION_FAILURE"
            }
            Self::NormalizationError(NormalizationError::SpecifiedOperationNotFound {
                operation_name: _,
            }) => "OPERATION_RESOLUTION_FAILURE",
            Self::NormalizationError(NormalizationError::MultipleMatchingOperationsFound) => {
                "OPERATION_RESOLUTION_FAILURE"
            }
            _ => "BAD_REQUEST",
        }
    }

    pub fn graphql_error_message(&self) -> String {
        match self {
            Self::PlannerError(_) | Self::InternalServiceError(_) => {
                return "Unexpected error".to_string()
            }
            _ => {}
        }

        self.to_string()
    }

    pub fn default_status_code(&self, prefer_ok: bool) -> StatusCode {
        match (self, prefer_ok) {
            (Self::InternalServiceError(_), _) => StatusCode::INTERNAL_SERVER_ERROR,
            (Self::PlannerError(_), _) => StatusCode::INTERNAL_SERVER_ERROR,
            (Self::UnsupportedHttpMethod(_), _) => StatusCode::METHOD_NOT_ALLOWED,
            (Self::FailedToReadBodyBytes(_), _) => StatusCode::BAD_REQUEST,
            (Self::InvalidHeaderValue(_), _) => StatusCode::BAD_REQUEST,
            (Self::GetInvalidQueryParams(_), _) => StatusCode::BAD_REQUEST,
            (Self::GetMissingQueryParam(_), _) => StatusCode::BAD_REQUEST,
            (Self::FailedToParseBody(_), _) => StatusCode::BAD_REQUEST,
            (Self::FailedToParseVariables(_), _) => StatusCode::BAD_REQUEST,
            (Self::FailedToParseExtensions(_), _) => StatusCode::BAD_REQUEST,
            (Self::FailedToParseOperation(_), false) => StatusCode::BAD_REQUEST,
            (Self::FailedToParseOperation(_), true) => StatusCode::OK,
            (Self::NormalizationError(_), _) => StatusCode::BAD_REQUEST,
            (Self::VariablesCoercionError(_), false) => StatusCode::BAD_REQUEST,
            (Self::VariablesCoercionError(_), true) => StatusCode::OK,
            (Self::MutationNotAllowedOverHttpGet, _) => StatusCode::METHOD_NOT_ALLOWED,
            (Self::ValidationErrors(_), true) => StatusCode::OK,
            (Self::ValidationErrors(_), false) => StatusCode::BAD_REQUEST,
            (Self::MissingContentTypeHeader, _) => StatusCode::NOT_ACCEPTABLE,
            (Self::UnsupportedContentType, _) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        }
    }
}

impl From<PipelineError> for PipelineErrorVariant {
    fn from(error: PipelineError) -> Self {
        error.error
    }
}

impl From<PipelineErrorVariant> for PipelineError {
    fn from(error: PipelineErrorVariant) -> Self {
        Self {
            error,
            accept_header: None,
        }
    }
}

impl IntoResponse for PipelineError {
    fn into_response(self) -> Response<Body> {
        let accept_ok = &self
            .accept_header
            .is_some_and(|v| v.contains(APPLICATION_JSON.to_str().unwrap()));
        let status = self.error.default_status_code(*accept_ok);

        if let PipelineErrorVariant::ValidationErrors(validation_errors) = self.error {
            let validation_error_result = ExecutionResult {
                data: None,
                errors: Some(validation_errors.iter().map(|error| error.into()).collect()),
                extensions: None,
            };

            return (
                status,
                serde_json::to_string(&validation_error_result).unwrap(),
            )
                .into_response();
        }

        let code = self.error.graphql_error_code();
        let message = self.error.graphql_error_message();

        let graphql_error = GraphQLError {
            extensions: Some(HashMap::from([(
                "code".to_string(),
                Value::String(code.to_string()),
            )])),
            message,
            path: None,
            locations: None,
        };

        let result = ExecutionResult {
            data: None,
            errors: Some(vec![graphql_error]),
            extensions: None,
        };

        (status, serde_json::to_string(&result).unwrap()).into_response()
    }
}
