use std::sync::Arc;

use graphql_tools::validation::utils::ValidationError;
use hive_router_plan_executor::{
    execution::{error::PlanExecutionError, jwt_forward::JwtForwardingError},
    response::graphql_error::GraphQLError,
};
use hive_router_query_planner::{
    ast::normalization::error::NormalizationError, planner::PlannerError,
};
use http::{HeaderName, Method, StatusCode};
use ntex::web::error::QueryPayloadError;
use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

use crate::{
    jwt::errors::JwtError,
    pipeline::{authorization::AuthorizationError, progressive_override::LabelEvaluationError},
};

#[derive(Debug, thiserror::Error, IntoStaticStr)]
pub enum PipelineError {
    // HTTP-related errors
    #[error("Unsupported HTTP method: {0}")]
    #[strum(serialize = "METHOD_NOT_ALLOWED")]
    UnsupportedHttpMethod(Method),
    #[error("Header '{0}' has invalid value")]
    #[strum(serialize = "INVALID_HEADER")]
    InvalidHeaderValue(HeaderName),
    #[error("Content-Type header is missing")]
    #[strum(serialize = "MISSING_CONTENT_TYPE_HEADER")]
    MissingContentTypeHeader,
    #[error("Content-Type header is not supported")]
    #[strum(serialize = "UNSUPPORTED_CONTENT_TYPE")]
    UnsupportedContentType,

    // GET Specific pipeline errors
    #[error("Failed to deserialize query parameters")]
    #[strum(serialize = "INVALID_QUERY_PARAMS")]
    GetInvalidQueryParams,
    #[error("Missing query parameter: {0}")]
    #[strum(serialize = "MISSING_QUERY_PARAM")]
    GetMissingQueryParam(&'static str),
    #[error("Cannot perform mutations over GET")]
    #[strum(serialize = "MUTATION_NOT_ALLOWED_OVER_HTTP_GET")]
    MutationNotAllowedOverHttpGet,
    #[error("Failed to parse query parameters")]
    #[strum(serialize = "UNPROCESSABLE_QUERY_PARAMS")]
    GetUnprocessableQueryParams(QueryPayloadError),

    // GraphQL-specific errors
    #[error("Failed to parse GraphQL request payload")]
    #[strum(serialize = "BAD_REQUEST")]
    FailedToParseBody(sonic_rs::Error),
    #[error("Failed to parse GraphQL variables JSON")]
    #[strum(serialize = "BAD_REQUEST")]
    FailedToParseVariables(sonic_rs::Error),
    #[error("Failed to parse GraphQL extensions JSON")]
    #[strum(serialize = "BAD_REQUEST")]
    FailedToParseExtensions(sonic_rs::Error),
    #[error("Failed to parse GraphQL operation: {0}")]
    #[strum(serialize = "GRAPHQL_PARSE_FAILED")]
    FailedToParseOperation(graphql_tools::parser::query::ParseError),
    #[error("Failed to normalize GraphQL operation")]
    #[strum(serialize = "OPERATION_RESOLUTION_FAILURE")]
    NormalizationError(NormalizationError),
    #[error("Failed to collect GraphQL variables: {0}")]
    #[strum(serialize = "BAD_USER_INPUT")]
    VariablesCoercionError(String),
    #[error("Validation errors")]
    #[strum(serialize = "GRAPHQL_VALIDATION_FAILED")]
    ValidationErrors(Arc<Vec<ValidationError>>),
    #[error("Authorization failed")]
    #[strum(serialize = "UNAUTHORIZED_OPERATION")]
    AuthorizationFailed(Vec<AuthorizationError>),
    #[error("Failed to execute a plan: {0}")]
    #[strum(serialize = "PLAN_EXECUTION_FAILED")]
    PlanExecutionError(PlanExecutionError),
    #[error("Failed to produce a plan: {0}")]
    #[strum(serialize = "QUERY_PLAN_BUILD_FAILED")]
    PlannerError(Arc<PlannerError>),
    #[error(transparent)]
    #[strum(serialize = "OVERRIDE_LABEL_EVALUATION_FAILED")]
    LabelEvaluationError(LabelEvaluationError),

    // HTTP Security-related errors
    #[error("Required CSRF header(s) not present")]
    #[strum(serialize = "CSRF_PREVENTION_FAILED")]
    CsrfPreventionFailed,

    // JWT-auth plugin errors
    #[error(transparent)]
    #[strum(serialize = "JWT_ERROR")]
    JwtError(JwtError),
    #[error("Failed to forward jwt: {0}")]
    #[strum(serialize = "JWT_FORWARDING_ERROR")]
    JwtForwardingError(JwtForwardingError),
}

impl PipelineError {
    pub fn graphql_error_code(&self) -> &'static str {
        match self {
            Self::JwtError(err) => err.error_code(),
            Self::PlanExecutionError(err) => err.error_code(),
            _ => self.into(),
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
            (Self::AuthorizationFailed(_), _) => StatusCode::FORBIDDEN,
            (Self::MissingContentTypeHeader, _) => StatusCode::NOT_ACCEPTABLE,
            (Self::UnsupportedContentType, _) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            (Self::CsrfPreventionFailed, _) => StatusCode::FORBIDDEN,
            (Self::JwtError(err), _) => err.status_code(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FailedExecutionResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<GraphQLError>>,
}
