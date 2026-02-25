use std::{sync::Arc, vec};

use graphql_tools::validation::utils::ValidationError;
use hive_router_plan_executor::{
    execution::{error::PlanExecutionError, jwt_forward::JwtForwardingError},
    headers::errors::HeaderRuleRuntimeError,
    hooks::on_graphql_error::handle_graphql_errors_with_plugins,
    response::graphql_error::GraphQLError,
};
use hive_router_query_planner::{
    ast::normalization::error::NormalizationError, planner::PlannerError,
};
use http::{header::RETRY_AFTER, HeaderName, Method, StatusCode};
use ntex::{
    http::ResponseBuilder,
    web::{self, error::QueryPayloadError},
};
use serde::Serialize;
use strum::IntoStaticStr;

use crate::{
    jwt::errors::JwtError,
    pipeline::{
        authorization::AuthorizationError, body_read::ReadBodyStreamError, header::RequestAccepts,
        progressive_override::LabelEvaluationError,
    },
    RouterSharedState,
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
    GetUnprocessableQueryParams(#[from] QueryPayloadError),

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
    FailedToParseOperation(#[from] graphql_tools::parser::query::ParseError),
    #[error("Failed to minify parsed GraphQL operation: {0}")]
    #[strum(serialize = "GRAPHQL_PARSE_MINIFY_FAILED")]
    FailedToMinifyParsedOperation(String),
    #[error("Failed to normalize GraphQL operation")]
    #[strum(serialize = "OPERATION_RESOLUTION_FAILURE")]
    NormalizationError(#[from] NormalizationError),
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
    PlanExecutionError(#[from] PlanExecutionError),
    #[error("Failed to produce a plan: {0}")]
    #[strum(serialize = "QUERY_PLAN_BUILD_FAILED")]
    PlannerError(#[from] Arc<PlannerError>),
    #[error(transparent)]
    #[strum(serialize = "OVERRIDE_LABEL_EVALUATION_FAILED")]
    LabelEvaluationError(#[from] LabelEvaluationError),

    // HTTP Security-related errors
    #[error("Required CSRF header(s) not present")]
    #[strum(serialize = "CSRF_PREVENTION_FAILED")]
    CsrfPreventionFailed,

    // JWT-auth plugin errors
    #[error(transparent)]
    #[strum(serialize = "JWT_ERROR")]
    JwtError(#[from] JwtError),
    #[error("Failed to forward jwt: {0}")]
    #[strum(serialize = "JWT_FORWARDING_ERROR")]
    JwtForwardingError(#[from] JwtForwardingError),

    // Introspection permission errors
    #[error("Failed to evaluate introspection expression: {0}")]
    #[strum(serialize = "INTROSPECTION_PERMISSION_EVALUATION_ERROR")]
    IntrospectionPermissionEvaluationError(String),
    #[error("Introspection queries are disabled")]
    #[strum(serialize = "INTROSPECTION_DISABLED")]
    IntrospectionDisabled,

    // Subscription-related errors
    #[error("Subscriptions are not supported")]
    #[strum(serialize = "SUBSCRIPTIONS_NOT_SUPPORTED")]
    SubscriptionsNotSupported,
    #[error("Subscriptions are not supported over accepted transport(s)")]
    #[strum(serialize = "SUBSCRIPTIONS_TRANSPORT_NOT_SUPPORTED")]
    SubscriptionsTransportNotSupported,

    #[error(transparent)]
    #[strum(serialize = "READ_BODY_STREAM_ERROR")]
    ReadBodyStreamError(#[from] ReadBodyStreamError),

    #[error("Request timed out")]
    #[strum(serialize = "GATEWAY_TIMEOUT")]
    TimeoutError,

    #[error(transparent)]
    #[strum(serialize = "HEADER_PROPAGATION_FAILURE")]
    HeaderPropagation(#[from] HeaderRuleRuntimeError),

    #[error("Failed to serialize the query plan: {0}")]
    #[strum(serialize = "QUERY_PLAN_SERIALIZATION_FAILED")]
    QueryPlanSerializationFailed(sonic_rs::Error),

    #[error("No supergraph available yet, unable to process request")]
    #[strum(serialize = "NO_SUPERGRAPH_AVAILABLE")]
    NoSupergraphAvailable,
}

impl PipelineError {
    pub fn graphql_error_code(&self) -> &'static str {
        match self {
            Self::JwtError(err) => err.error_code(),
            Self::PlanExecutionError(err) => err.error_code(),
            Self::ReadBodyStreamError(err) => err.error_code(),
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
            (Self::FailedToMinifyParsedOperation(_), false) => StatusCode::BAD_REQUEST,
            (Self::FailedToMinifyParsedOperation(_), true) => StatusCode::OK,
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
            (Self::IntrospectionPermissionEvaluationError(_), _) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            (Self::IntrospectionDisabled, _) => StatusCode::FORBIDDEN,
            (Self::SubscriptionsNotSupported, _) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            (Self::SubscriptionsTransportNotSupported, _) => StatusCode::NOT_ACCEPTABLE,
            (Self::ReadBodyStreamError(err), _) => err.status_code(),
            (Self::TimeoutError, _) => StatusCode::GATEWAY_TIMEOUT,
            (Self::HeaderPropagation(_), _) => StatusCode::INTERNAL_SERVER_ERROR,
            (Self::QueryPlanSerializationFailed(_), _) => StatusCode::INTERNAL_SERVER_ERROR,
            (Self::NoSupergraphAvailable, _) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl From<&PipelineError> for GraphQLError {
    fn from(val: &PipelineError) -> Self {
        let code = val.graphql_error_code();
        let message = val.graphql_error_message();
        GraphQLError::from_message_and_code(message, code)
    }
}

#[derive(Serialize)]
struct FailedExecutionResult {
    errors: Vec<GraphQLError>,
}

impl web::error::WebResponseError for PipelineError {
    fn error_response(&self, req: &web::HttpRequest) -> web::HttpResponse {
        // Retrieve the negotiated response mode, defaulting to standard if not set
        let response_mode = req.get_response_mode();
        let single_content_type = response_mode.single_content_type();

        let prefer_ok = response_mode.prefer_status_ok_for_errors();

        let status = self.default_status_code(prefer_ok);

        let mut res = ResponseBuilder::new(status);

        if let Some(single_content_type) = single_content_type {
            res.content_type(single_content_type.as_ref());
        }

        let mut errors = match self {
            Self::ValidationErrors(validation_errors) => {
                validation_errors.iter().map(|error| error.into()).collect()
            }
            Self::AuthorizationFailed(authorization_errors) => authorization_errors
                .iter()
                .map(|error| error.into())
                .collect(),
            _ => {
                let graphql_error: GraphQLError = self.into();

                if matches!(self, PipelineError::NoSupergraphAvailable) {
                    res.header(RETRY_AFTER, "10");
                }

                vec![graphql_error]
            }
        };

        if let Some(plugins) = req
            .app_state::<Arc<RouterSharedState>>()
            .and_then(|shared_state| shared_state.plugins.clone())
        {
            let (new_errors, new_status_code) =
                handle_graphql_errors_with_plugins(&plugins, errors, status);
            errors = new_errors;
            res.status(new_status_code);
        }

        res.json(&FailedExecutionResult { errors })
    }
}
