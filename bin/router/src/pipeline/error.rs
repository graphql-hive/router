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
    http::ResponseBuilder,
    web::{self, error::QueryPayloadError},
};
use serde::{Deserialize, Serialize};

use crate::{
    jwt::errors::JwtError,
    pipeline::{
        authorization::AuthorizationError, header::SingleContentType,
        progressive_override::LabelEvaluationError,
    },
};

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
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
    FailedToParseOperation(graphql_tools::parser::query::ParseError),
    #[error("Failed to normalize GraphQL operation")]
    NormalizationError(NormalizationError),
    #[error("Failed to collect GraphQL variables: {0}")]
    VariablesCoercionError(String),
    #[error("Validation errors")]
    ValidationErrors(Arc<Vec<ValidationError>>),
    #[error("Authorization failed")]
    AuthorizationFailed(Vec<AuthorizationError>),
    #[error("Failed to execute a plan: {0}")]
    PlanExecutionError(PlanExecutionError),
    #[error("Failed to produce a plan: {0}")]
    PlannerError(Arc<PlannerError>),
    #[error(transparent)]
    LabelEvaluationError(LabelEvaluationError),

    // HTTP Security-related errors
    #[error("Required CSRF header(s) not present")]
    CsrfPreventionFailed,

    // JWT-auth plugin errors
    #[error(transparent)]
    JwtError(JwtError),
    #[error("Failed to forward jwt: {0}")]
    JwtForwardingError(JwtForwardingError),

    // Subscription-related errors
    #[error("Subscriptions are not supported")]
    SubscriptionsNotSupported,
    #[error("Subscriptions are not supported over accepted transport(s)")]
    SubscriptionsTransportNotSupported,
}

impl PipelineError {
    pub fn graphql_error_code(&self) -> &'static str {
        match self {
            Self::UnsupportedHttpMethod(_) => "METHOD_NOT_ALLOWED",
            Self::PlannerError(_) => "QUERY_PLAN_BUILD_FAILED",
            Self::PlanExecutionError(_) => "QUERY_PLAN_EXECUTION_FAILED",
            Self::LabelEvaluationError(_) => "OVERRIDE_LABEL_EVALUATION_FAILED",
            Self::FailedToParseOperation(_) => "GRAPHQL_PARSE_FAILED",
            Self::ValidationErrors(_) => "GRAPHQL_VALIDATION_FAILED",
            Self::VariablesCoercionError(_) => "BAD_USER_INPUT",
            Self::AuthorizationFailed(_) => "UNAUTHORIZED_OPERATION",
            Self::NormalizationError(NormalizationError::OperationNotFound) => {
                "OPERATION_RESOLUTION_FAILURE"
            }
            Self::NormalizationError(NormalizationError::SpecifiedOperationNotFound {
                operation_name: _,
            }) => "OPERATION_RESOLUTION_FAILURE",
            Self::NormalizationError(NormalizationError::MultipleMatchingOperationsFound) => {
                "OPERATION_RESOLUTION_FAILURE"
            }
            Self::JwtError(err) => err.error_code(),
            Self::SubscriptionsNotSupported => "SUBSCRIPTIONS_NOT_SUPPORTED",
            Self::SubscriptionsTransportNotSupported => "SUBSCRIPTIONS_TRANSPORT_NOT_SUPPORTED",
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
            (Self::AuthorizationFailed(_), _) => StatusCode::FORBIDDEN,
            (Self::MissingContentTypeHeader, _) => StatusCode::NOT_ACCEPTABLE,
            (Self::UnsupportedContentType, _) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            (Self::CsrfPreventionFailed, _) => StatusCode::FORBIDDEN,
            (Self::JwtError(err), _) => err.status_code(),
            (Self::SubscriptionsNotSupported, _) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            (Self::SubscriptionsTransportNotSupported, _) => StatusCode::NOT_ACCEPTABLE,
        }
    }

    pub fn into_response(self, content_type: Option<SingleContentType>) -> web::HttpResponse {
        let prefer_ok = content_type.unwrap_or_default() == SingleContentType::JSON;

        let status = self.default_status_code(prefer_ok);

        if let PipelineError::ValidationErrors(validation_errors) = self {
            let validation_error_result = FailedExecutionResult {
                errors: Some(validation_errors.iter().map(|error| error.into()).collect()),
            };

            return ResponseBuilder::new(status).json(&validation_error_result);
        }

        if let PipelineError::AuthorizationFailed(authorization_errors) = self {
            let authorization_error_result = FailedExecutionResult {
                errors: Some(
                    authorization_errors
                        .iter()
                        .map(|error| error.into())
                        .collect(),
                ),
            };

            return ResponseBuilder::new(status).json(&authorization_error_result);
        }

        let code = self.graphql_error_code();
        let message = self.graphql_error_message();

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

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FailedExecutionResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<GraphQLError>>,
}
