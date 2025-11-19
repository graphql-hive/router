use std::sync::Arc;

use graphql_tools::validation::utils::ValidationError;
use hive_router_plan_executor::{
    execution::{error::PlanExecutionError, jwt_forward::JwtForwardingError},
    response::graphql_error::{GraphQLError, GraphQLErrorExtensions},
};
use hive_router_query_planner::{
    ast::normalization::error::NormalizationError, planner::PlannerError,
};
use http::{HeaderName, Method, StatusCode};
use ntex::{
    http::{Response, ResponseBuilder},
    web::error::QueryPayloadError,
};
use serde::{Deserialize, Serialize};

use crate::pipeline::progressive_override::LabelEvaluationError;

#[derive(Debug)]
pub struct PipelineError {
    pub accept_ok: bool,
    pub error: PipelineErrorVariant,
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineErrorVariant {
    // HTTP-related errors
    #[error("Unsupported HTTP method: {0}")]
    UnsupportedHttpMethod(Method),
    #[error("Header '{0}' has invalid value")]
    InvalidHeaderValue(HeaderName),
    #[error("Content-Type header is missing")]
    MissingContentTypeHeader,
    #[error("Content-Type header is not supported")]
    UnsupportedContentType,

    // GET Specific pipeline errors
    #[error("Failed to deserialize query parameters")]
    GetInvalidQueryParams,
    #[error("Missing query parameter: {0}")]
    GetMissingQueryParam(&'static str),
    #[error("Cannot perform mutations over GET")]
    MutationNotAllowedOverHttpGet,
    #[error("Failed to parse query parameters")]
    GetUnprocessableQueryParams(QueryPayloadError),

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
    #[error("Failed to execute a plan: {0}")]
    PlanExecutionError(PlanExecutionError),
    #[error("Failed to produce a plan: {0}")]
    PlannerError(PlannerError),
    #[error(transparent)]
    LabelEvaluationError(LabelEvaluationError),

    // HTTP Security-related errors
    #[error("Required CSRF header(s) not present")]
    CsrfPreventionFailed,

    // JWT-auth plugin errors
    #[error("Failed to forward jwt: {0}")]
    JwtForwardingError(JwtForwardingError),
}

impl PipelineErrorVariant {
    pub fn graphql_error_code(&self) -> &'static str {
        match self {
            Self::UnsupportedHttpMethod(_) => "METHOD_NOT_ALLOWED",
            Self::PlannerError(_) => "QUERY_PLAN_BUILD_FAILED",
            Self::PlanExecutionError(_) => "QUERY_PLAN_EXECUTION_FAILED",
            Self::LabelEvaluationError(_) => "OVERRIDE_LABEL_EVALUATION_FAILED",
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
            Self::PlannerError(_) => "Unexpected error".to_string(),
            _ => self.to_string(),
        }
    }

    pub fn default_status_code(&self, prefer_ok: bool) -> StatusCode {
        match (self, prefer_ok) {
            (Self::PlannerError(_), _) => StatusCode::INTERNAL_SERVER_ERROR,
            (Self::PlanExecutionError(_), _) => StatusCode::INTERNAL_SERVER_ERROR,
            (Self::LabelEvaluationError(_), _) => StatusCode::INTERNAL_SERVER_ERROR,
            (Self::JwtForwardingError(_), _) => StatusCode::INTERNAL_SERVER_ERROR,
            (Self::UnsupportedHttpMethod(_), _) => StatusCode::METHOD_NOT_ALLOWED,
            (Self::InvalidHeaderValue(_), _) => StatusCode::BAD_REQUEST,
            (Self::GetUnprocessableQueryParams(_), _) => StatusCode::BAD_REQUEST,
            (Self::GetInvalidQueryParams, _) => StatusCode::BAD_REQUEST,
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
            (Self::CsrfPreventionFailed, _) => StatusCode::FORBIDDEN,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FailedExecutionResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<GraphQLError>>,
}

impl From<PipelineError> for Response {
    fn from(val: PipelineError) -> Self {
        let status = val.error.default_status_code(val.accept_ok);

        if let PipelineErrorVariant::ValidationErrors(validation_errors) = val.error {
            let validation_error_result = FailedExecutionResult {
                errors: Some(validation_errors.iter().map(|error| error.into()).collect()),
            };

            return ResponseBuilder::new(status).json(&validation_error_result);
        }

        let code = val.error.graphql_error_code();
        let message = val.error.graphql_error_message();

        let graphql_error = GraphQLError::from_message_and_extensions(
            message,
            GraphQLErrorExtensions::new_from_code(code),
        );

        let result = FailedExecutionResult {
            errors: Some(vec![graphql_error]),
        };

        ResponseBuilder::new(status).json(&result)
    }
}
