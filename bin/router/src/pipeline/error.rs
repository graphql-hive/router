use std::{sync::Arc, vec};

use futures_util::stream;
use graphql_tools::validation::utils::ValidationError;
use hive_console_sdk::expressions::{
    values::boolean::BooleanConversionError, ProgramResolutionError,
};
use hive_router_internal::http::ReadBodyStreamError;
use hive_router_internal::telemetry::logging::{summary, targets};
use hive_router_plan_executor::variables::VariableCoercionError;
use hive_router_plan_executor::{
    coprocessor::CoprocessorError,
    execution::{
        error::PlanExecutionError, jwt_forward::JwtForwardingError, plan::FailedExecutionResult,
    },
    headers::errors::HeaderRuleRuntimeError,
    hooks::on_graphql_error::handle_graphql_errors_with_plugins,
    operation_filter::OperationFilterError,
    plugin_context::PluginContext,
    request_context::{RequestContextError, RequestContextExt},
    response::graphql_error::GraphQLError,
};
use hive_router_query_planner::{
    ast::normalization::error::NormalizationError, planner::PlannerError,
};
use http::{header, HeaderValue};
use http::{HeaderName, Method, StatusCode};
use ntex::{
    http::ResponseBuilder,
    web::{self, error::QueryPayloadError, HttpRequest},
};
use strum::IntoStaticStr;
use tracing::error;

use crate::{
    jwt::errors::JwtError,
    pipeline::{
        authorization::AuthorizationError,
        header::{ResponseMode, StreamContentType},
        multipart_subscribe::{
            self, APOLLO_MULTIPART_HTTP_CONTENT_TYPE, INCREMENTAL_DELIVERY_CONTENT_TYPE,
        },
        progressive_override::LabelEvaluationError,
        sse,
    },
    schema_state::RouterSupergraphRuntimeError,
    RouterSharedState,
};

pub type PipelineErrorAdditionalHeaders = Vec<(HeaderName, HeaderValue)>;

/// Errors caused by the client's own request (bad input, unauthenticated/unauthorized,
/// unsupported transport, etc).
/// Their `Display` message is safe to return to the client as-is because it's derived only from client-controlled data.
#[derive(Debug, thiserror::Error, IntoStaticStr)]
pub enum ClientPipelineError {
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

    #[error("Request headers exceed the maximum allowed size")]
    #[strum(serialize = "REQUEST_HEADER_FIELDS_TOO_LARGE")]
    RequestHeadersTooLarge,

    #[error("Missing query parameter: {0}")]
    #[strum(serialize = "MISSING_QUERY_PARAM")]
    GetMissingQueryParam(&'static str),

    #[error("Cannot perform mutations over GET")]
    #[strum(serialize = "MUTATION_NOT_ALLOWED_OVER_HTTP_GET")]
    MutationNotAllowedOverHttpGet,

    #[error("Failed to parse query parameters")]
    #[strum(serialize = "UNPROCESSABLE_QUERY_PARAMS")]
    GetUnprocessableQueryParams(QueryPayloadError),

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
    FailedToParseOperation(Arc<graphql_tools::parser::query::ParseError>),

    #[error("Persisted document not found: {0}")]
    #[strum(serialize = "PERSISTED_DOCUMENT_NOT_FOUND")]
    PersistedDocumentNotFound(String),

    #[error("Persisted document id is required")]
    #[strum(serialize = "PERSISTED_DOCUMENT_ID_REQUIRED")]
    PersistedDocumentIdRequired,

    #[error("{0}")]
    #[strum(serialize = "PERSISTED_DOCUMENT_EXTRACTION_FAILED")]
    PersistedDocumentExtraction(String),

    #[error("Failed to normalize GraphQL operation")]
    #[strum(serialize = "OPERATION_RESOLUTION_FAILURE")]
    NormalizationError(Arc<NormalizationError>),

    #[error(transparent)]
    #[strum(serialize = "BAD_USER_INPUT")]
    VariablesCoercionError(VariableCoercionError),

    #[error("Validation errors")]
    #[strum(serialize = "GRAPHQL_VALIDATION_FAILED")]
    ValidationErrors(Arc<Vec<ValidationError>>),

    #[error("Authorization failed")]
    #[strum(serialize = "UNAUTHORIZED_OPERATION")]
    AuthorizationFailed(Vec<AuthorizationError>),

    #[error("Required CSRF header(s) not present")]
    #[strum(serialize = "CSRF_PREVENTION_FAILED")]
    CsrfPreventionFailed,

    #[error(transparent)]
    #[strum(serialize = "JWT_ERROR")]
    JwtError(JwtError),

    #[error("Introspection queries are disabled")]
    #[strum(serialize = "INTROSPECTION_DISABLED")]
    IntrospectionDisabled,

    #[error("Subscriptions are not supported")]
    #[strum(serialize = "SUBSCRIPTIONS_NOT_SUPPORTED")]
    SubscriptionsNotSupported,

    #[error("Subscriptions are not supported over accepted transport(s)")]
    #[strum(serialize = "SUBSCRIPTIONS_TRANSPORT_NOT_SUPPORTED")]
    SubscriptionsTransportNotSupported,

    #[error(transparent)]
    #[strum(serialize = "READ_BODY_STREAM_ERROR")]
    ReadBodyStreamError(ReadBodyStreamError),

    #[error("Request timed out")]
    #[strum(serialize = "GATEWAY_TIMEOUT")]
    TimeoutError,

    #[error("Operation estimated cost exceeds max cost")]
    #[strum(serialize = "COST_ESTIMATED_TOO_EXPENSIVE")]
    CostEstimatedTooExpensive {
        response_headers: PipelineErrorAdditionalHeaders,
    },

    #[error(
        "Exactly one slicing argument is required for field '{field_name}', but found {found}"
    )]
    #[strum(serialize = "COST_INVALID_SLICING_ARGUMENTS")]
    CostInvalidSlicingArguments { field_name: String, found: usize },
}

/// Errors caused by a bug or infrastructure failure on the router's side.
/// Their `Display` message may contain internal details (subgraph URLs, storage/network errors, VRL diagnostics) and
/// must never reach the client - only the generic message from `graphql_error_message()`
/// does. The real message is still logged for debugging purposes.
#[derive(Debug, thiserror::Error, IntoStaticStr)]
pub enum InternalPipelineError {
    #[error("Failed to produce a plan: {0}")]
    #[strum(serialize = "QUERY_PLAN_BUILD_FAILED")]
    PlannerError(Arc<PlannerError>),

    #[error("Failed to minify parsed GraphQL operation: {0}")]
    #[strum(serialize = "GRAPHQL_PARSE_MINIFY_FAILED")]
    FailedToMinifyParsedOperation(String),

    #[error("No supergraph available yet, unable to process request")]
    #[strum(serialize = "NO_SUPERGRAPH_AVAILABLE")]
    NoSupergraphAvailable {
        response_headers: PipelineErrorAdditionalHeaders,
    },

    #[error("Failed to execute a plan: {0}")]
    #[strum(serialize = "PLAN_EXECUTION_FAILED")]
    PlanExecutionError(PlanExecutionError),

    #[error(transparent)]
    #[strum(serialize = "OVERRIDE_LABEL_EVALUATION_FAILED")]
    LabelEvaluationError(LabelEvaluationError),

    #[error("Failed to forward jwt: {0}")]
    #[strum(serialize = "JWT_FORWARDING_ERROR")]
    JwtForwardingError(JwtForwardingError),

    #[error("{0}")]
    #[strum(serialize = "PERSISTED_DOCUMENT_RESOLUTION_FAILED")]
    PersistedDocumentResolution(String),

    #[error("Failed to evaluate persisted document require_id expression: {0}")]
    #[strum(serialize = "PERSISTED_DOCUMENT_ID_EXPRESSION_EVALUATION_ERROR")]
    PersistedDocumentIdExpressionEvaluationError(ProgramResolutionError<BooleanConversionError>),

    #[error("Failed to evaluate introspection expression: {0}")]
    #[strum(serialize = "INTROSPECTION_PERMISSION_EVALUATION_ERROR")]
    IntrospectionPermissionEvaluationError(String),

    #[error(transparent)]
    #[strum(serialize = "HEADER_PROPAGATION_FAILURE")]
    HeaderPropagation(HeaderRuleRuntimeError),

    #[error("Failed to serialize the query plan: {0}")]
    #[strum(serialize = "QUERY_PLAN_SERIALIZATION_FAILED")]
    QueryPlanSerializationFailed(sonic_rs::Error),

    #[error(transparent)]
    CoprocessorError(CoprocessorError),

    #[error("Request context error")]
    RequestContextError(RequestContextError),

    #[error(transparent)]
    OperationFilterFailed(OperationFilterError),

    #[error("Supergraph runtime error")]
    #[strum(serialize = "SUPERGRAPH_RUNTIME_ERROR")]
    RouterSupergraphRuntimeError(RouterSupergraphRuntimeError),
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error(transparent)]
    Client(ClientPipelineError),
    #[error(transparent)]
    Internal(InternalPipelineError),
}

impl From<ClientPipelineError> for PipelineError {
    fn from(value: ClientPipelineError) -> Self {
        PipelineError::Client(value)
    }
}

impl From<InternalPipelineError> for PipelineError {
    fn from(value: InternalPipelineError) -> Self {
        PipelineError::Internal(value)
    }
}

impl From<QueryPayloadError> for PipelineError {
    fn from(value: QueryPayloadError) -> Self {
        ClientPipelineError::GetUnprocessableQueryParams(value).into()
    }
}

impl From<Arc<NormalizationError>> for PipelineError {
    fn from(value: Arc<NormalizationError>) -> Self {
        ClientPipelineError::NormalizationError(value).into()
    }
}

impl From<JwtError> for PipelineError {
    fn from(value: JwtError) -> Self {
        ClientPipelineError::JwtError(value).into()
    }
}

impl From<ReadBodyStreamError> for PipelineError {
    fn from(value: ReadBodyStreamError) -> Self {
        ClientPipelineError::ReadBodyStreamError(value).into()
    }
}

impl From<Arc<PlannerError>> for PipelineError {
    fn from(value: Arc<PlannerError>) -> Self {
        InternalPipelineError::PlannerError(value).into()
    }
}

impl From<PlanExecutionError> for PipelineError {
    fn from(value: PlanExecutionError) -> Self {
        InternalPipelineError::PlanExecutionError(value).into()
    }
}

impl From<LabelEvaluationError> for PipelineError {
    fn from(value: LabelEvaluationError) -> Self {
        InternalPipelineError::LabelEvaluationError(value).into()
    }
}

impl From<JwtForwardingError> for PipelineError {
    fn from(value: JwtForwardingError) -> Self {
        InternalPipelineError::JwtForwardingError(value).into()
    }
}

impl From<CoprocessorError> for PipelineError {
    fn from(value: CoprocessorError) -> Self {
        InternalPipelineError::CoprocessorError(value).into()
    }
}

impl From<RequestContextError> for PipelineError {
    fn from(value: RequestContextError) -> Self {
        InternalPipelineError::RequestContextError(value).into()
    }
}

impl From<OperationFilterError> for PipelineError {
    fn from(value: OperationFilterError) -> Self {
        InternalPipelineError::OperationFilterFailed(value).into()
    }
}

impl From<RouterSupergraphRuntimeError> for PipelineError {
    fn from(value: RouterSupergraphRuntimeError) -> Self {
        InternalPipelineError::RouterSupergraphRuntimeError(value).into()
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ParserCacheError {
    #[error("Failed to parse GraphQL operation: {0}")]
    ParseError(Arc<graphql_tools::parser::query::ParseError>),
    #[error("Failed to minify parsed GraphQL operation: {0}")]
    MinifyError(String),
    #[error("Validation errors")]
    ValidationErrors(Arc<Vec<ValidationError>>),
}

impl From<Arc<ParserCacheError>> for PipelineError {
    fn from(value: Arc<ParserCacheError>) -> Self {
        match value.as_ref() {
            ParserCacheError::ParseError(err) => {
                ClientPipelineError::FailedToParseOperation(err.clone()).into()
            }
            ParserCacheError::MinifyError(err) => {
                InternalPipelineError::FailedToMinifyParsedOperation(err.clone()).into()
            }
            ParserCacheError::ValidationErrors(errs) => {
                ClientPipelineError::ValidationErrors(errs.clone()).into()
            }
        }
    }
}

impl ClientPipelineError {
    fn additional_response_headers(&self) -> Option<&PipelineErrorAdditionalHeaders> {
        match self {
            Self::CostEstimatedTooExpensive { response_headers } => Some(response_headers),
            _ => None,
        }
    }

    fn graphql_error_code(&self) -> &'static str {
        match self {
            Self::JwtError(err) => err.error_code(),
            Self::ReadBodyStreamError(err) => err.error_code(),
            _ => self.into(),
        }
    }

    fn default_status_code(&self, prefer_ok: bool) -> StatusCode {
        match (self, prefer_ok) {
            (Self::UnsupportedHttpMethod(_), _) => StatusCode::METHOD_NOT_ALLOWED,
            (Self::InvalidHeaderValue(_), _) => StatusCode::BAD_REQUEST,
            (Self::GetUnprocessableQueryParams(_), _) => StatusCode::BAD_REQUEST,
            (Self::GetMissingQueryParam(_), _) => StatusCode::BAD_REQUEST,
            (Self::FailedToParseBody(_), _) => StatusCode::BAD_REQUEST,
            (Self::FailedToParseVariables(_), _) => StatusCode::BAD_REQUEST,
            (Self::FailedToParseExtensions(_), _) => StatusCode::BAD_REQUEST,
            (Self::PersistedDocumentNotFound(_), false) => StatusCode::BAD_REQUEST,
            (Self::PersistedDocumentNotFound(_), true) => StatusCode::OK,
            (Self::PersistedDocumentIdRequired, false) => StatusCode::BAD_REQUEST,
            (Self::PersistedDocumentIdRequired, true) => StatusCode::OK,
            (Self::PersistedDocumentExtraction(_), false) => StatusCode::BAD_REQUEST,
            (Self::PersistedDocumentExtraction(_), true) => StatusCode::OK,
            (Self::FailedToParseOperation(_), false) => StatusCode::BAD_REQUEST,
            (Self::FailedToParseOperation(_), true) => StatusCode::OK,
            (Self::NormalizationError(_), _) => StatusCode::BAD_REQUEST,
            (Self::VariablesCoercionError(_), false) => StatusCode::BAD_REQUEST,
            (Self::VariablesCoercionError(_), true) => StatusCode::OK,
            (Self::MutationNotAllowedOverHttpGet, _) => StatusCode::METHOD_NOT_ALLOWED,
            (Self::ValidationErrors(_), true) => StatusCode::OK,
            (Self::ValidationErrors(_), false) => StatusCode::BAD_REQUEST,
            (Self::CostEstimatedTooExpensive { .. }, true) => StatusCode::OK,
            (Self::CostEstimatedTooExpensive { .. }, false) => StatusCode::BAD_REQUEST,
            (Self::CostInvalidSlicingArguments { .. }, true) => StatusCode::OK,
            (Self::CostInvalidSlicingArguments { .. }, false) => StatusCode::BAD_REQUEST,
            (Self::AuthorizationFailed(_), _) => StatusCode::FORBIDDEN,
            (Self::MissingContentTypeHeader, _) => StatusCode::NOT_ACCEPTABLE,
            (Self::UnsupportedContentType, _) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            (Self::RequestHeadersTooLarge, _) => StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            (Self::CsrfPreventionFailed, _) => StatusCode::FORBIDDEN,
            (Self::JwtError(err), _) => err.status_code(),
            (Self::IntrospectionDisabled, _) => StatusCode::FORBIDDEN,
            (Self::SubscriptionsNotSupported, _) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            (Self::SubscriptionsTransportNotSupported, _) => StatusCode::NOT_ACCEPTABLE,
            (Self::ReadBodyStreamError(err), _) => err.status_code(),
            (Self::TimeoutError, _) => StatusCode::GATEWAY_TIMEOUT,
        }
    }
}

impl InternalPipelineError {
    fn additional_response_headers(&self) -> Option<&PipelineErrorAdditionalHeaders> {
        match self {
            Self::NoSupergraphAvailable { response_headers } => Some(response_headers),
            _ => None,
        }
    }

    fn graphql_error_code(&self) -> &'static str {
        match self {
            Self::PlanExecutionError(err) => err.error_code(),
            Self::CoprocessorError(err) => err.error_code(),
            _ => self.into(),
        }
    }

    fn default_status_code(&self, prefer_ok: bool) -> StatusCode {
        match self {
            Self::NoSupergraphAvailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            _ if prefer_ok => StatusCode::OK,
            Self::CoprocessorError(e) => e.status_code(),
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl PipelineError {
    pub fn additional_response_headers(&self) -> Option<&PipelineErrorAdditionalHeaders> {
        match self {
            Self::Client(err) => err.additional_response_headers(),
            Self::Internal(err) => err.additional_response_headers(),
        }
    }

    pub fn graphql_error_code(&self) -> &'static str {
        match self {
            Self::Client(err) => err.graphql_error_code(),
            Self::Internal(err) => err.graphql_error_code(),
        }
    }

    /// The message returned to the client.
    ///
    /// Internal errors always get a generic message - their real `Display` is only ever logged, never serialized to the client.
    pub fn graphql_error_message(&self) -> String {
        match self {
            Self::Client(err) => err.to_string(),
            Self::Internal(_) => "Internal server error".to_string(),
        }
    }

    pub fn default_status_code(&self, prefer_ok: bool) -> StatusCode {
        match self {
            Self::Client(err) => err.default_status_code(prefer_ok),
            Self::Internal(err) => err.default_status_code(prefer_ok),
        }
    }
}

#[inline]
pub fn handle_pipeline_error(
    err: PipelineError,
    req: &HttpRequest,
    shared_state: &RouterSharedState,
    response_mode: &ResponseMode,
) -> web::HttpResponse {
    if let PipelineError::Internal(inner) = &err {
        error!(target: targets::CORE, error = %inner, "internal pipeline error");
    }

    let error_count = match &err {
        PipelineError::Client(ClientPipelineError::ValidationErrors(errors)) => errors.len() as u32,
        PipelineError::Client(ClientPipelineError::AuthorizationFailed(errors)) => {
            errors.len() as u32
        }
        _ => 1,
    };

    summary::record(|s| {
        s.set_response_code(err.graphql_error_code());
        s.set_error_count(error_count);
    });

    let status = if matches!(response_mode, ResponseMode::StreamOnly(_)) {
        // alwats status OK for streaming response modes, because we accept
        // the stream and then stream the error from within the stream by default
        StatusCode::OK
    } else {
        let prefer_ok = response_mode.prefer_status_ok_for_errors();
        err.default_status_code(prefer_ok)
    };

    let mut res = ResponseBuilder::new(status);

    if let Some(headers) = err.additional_response_headers() {
        for (name, value) in headers {
            res.header(name, value);
        }
    }

    let mut errors = match &err {
        PipelineError::Client(ClientPipelineError::ValidationErrors(validation_errors)) => {
            validation_errors.iter().map(|error| error.into()).collect()
        }
        PipelineError::Client(ClientPipelineError::AuthorizationFailed(authorization_errors)) => {
            authorization_errors
                .iter()
                .map(|error| error.into())
                .collect()
        }
        PipelineError::Client(ClientPipelineError::CostEstimatedTooExpensive { .. }) => {
            vec![GraphQLError::from_message_and_code(
                err.graphql_error_message(),
                "COST_ESTIMATED_TOO_EXPENSIVE",
            )]
        }
        _ => {
            let code = err.graphql_error_code();
            let message = err.graphql_error_message();
            let graphql_error = GraphQLError::from_message_and_code(message, code);

            vec![graphql_error]
        }
    };

    if let Some(plugins) = &shared_state.plugins {
        let plugin_context = req.extensions().get::<Arc<PluginContext>>().cloned();
        let request_context = req.read_request_context().ok();
        if let (Some(plugin_context), Some(request_context)) = (plugin_context, request_context) {
            let (new_errors, new_status_code) = handle_graphql_errors_with_plugins(
                plugins,
                plugin_context.as_ref(),
                &request_context,
                errors,
                status,
            );
            errors = new_errors;
            res.status(new_status_code);
        }
    }

    if let Some(error_recorder) = shared_state
        .telemetry_context
        .metrics
        .graphql
        .error_recorder()
    {
        error_recorder
            .record_errors(|| errors.iter().map(|error| error.extensions.code.as_deref()));
    }

    let data = FailedExecutionResult { errors }.serialize();

    match response_mode {
        ResponseMode::SingleOnly(content_type) | ResponseMode::Dual(content_type, _) => res
            .header(header::CONTENT_TYPE, content_type.as_ref())
            .body(data),
        ResponseMode::StreamOnly(StreamContentType::IncrementalDelivery) => res
            .header(
                header::CONTENT_TYPE,
                http::HeaderValue::from_static(INCREMENTAL_DELIVERY_CONTENT_TYPE),
            )
            .streaming(multipart_subscribe::create_incremental_delivery_stream(
                Box::pin(stream::once(async move { data })),
            )),
        ResponseMode::StreamOnly(StreamContentType::SSE) => res
            .header(
                header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/event-stream"),
            )
            .streaming(sse::create_stream(
                Box::pin(stream::once(async move { data })),
                std::time::Duration::from_secs(10),
            )),
        ResponseMode::StreamOnly(StreamContentType::ApolloMultipartHTTP) => res
            .header(
                header::CONTENT_TYPE,
                http::HeaderValue::from_static(APOLLO_MULTIPART_HTTP_CONTENT_TYPE),
            )
            .streaming(multipart_subscribe::create_apollo_multipart_http_stream(
                Box::pin(stream::once(async move { data })),
                std::time::Duration::from_secs(10),
            )),
        ResponseMode::Laboratory => {
            unreachable!(
                "Laboratory can not be a response mode because Laboratory requests can not execute operations"
            )
        }
    }
}
