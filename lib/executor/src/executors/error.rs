use http::uri::InvalidUri;
use strum::IntoStaticStr;

#[derive(thiserror::Error, Debug, IntoStaticStr)]
pub enum SubgraphExecutorError {
    #[error("Failed to parse endpoint \"{0}\" as URI: {1}")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_PARSE_FAILURE")]
    EndpointParseFailure(String, InvalidUri),
    #[error("Failed to compile VRL expression for subgraph '{0}'. Please check your VRL expression for syntax errors. Diagnostic: {1}")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_EXPRESSION_BUILD_FAILURE")]
    EndpointExpressionBuild(String, String),
    #[error("Failed to resolve VRL expression. Runtime error: {0}")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_EXPRESSION_RESOLUTION_FAILURE")]
    EndpointExpressionResolutionFailure(String),
    #[error("VRL expression resolved to a non-string value.")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_EXPRESSION_WRONG_TYPE")]
    EndpointExpressionWrongType,
    #[error(
        "Static endpoint not found for subgraph. This is an internal error and should not happen."
    )]
    #[strum(serialize = "SUBGRAPH_STATIC_ENDPOINT_NOT_FOUND")]
    StaticEndpointNotFound,
    #[error("Failed to build request to subgraph: {0}")]
    #[strum(serialize = "SUBGRAPH_REQUEST_BUILD_FAILURE")]
    RequestBuildFailure(#[from] http::Error),
    #[error("Failed to send request to subgraph: {0}")]
    #[strum(serialize = "SUBGRAPH_REQUEST_FAILURE")]
    RequestFailure(#[from] hyper_util::client::legacy::Error),
    #[error("Failed to receive response: {0}")]
    #[strum(serialize = "SUBGRAPH_RESPONSE_FAILURE")]
    ResponseFailure(#[from] hyper::Error),
    #[error("Received an empty response body from subgraph")]
    #[strum(serialize = "SUBGRAPH_EMPTY_RESPONSE_BODY")]
    EmptyResponseBody,
    #[error("Failed to serialize variable \"{0}\": {1}")]
    #[strum(serialize = "SUBGRAPH_VARIABLES_SERIALIZATION_FAILURE")]
    VariablesSerializationFailure(String, sonic_rs::Error),
    #[error("Failed to compile VRL expression for timeout for subgraph '{0}'. Please check your VRL expression for syntax errors. Diagnostic: {0}")]
    #[strum(serialize = "SUBGRAPH_TIMEOUT_EXPRESSION_BUILD_FAILURE")]
    RequestTimeoutExpressionBuild(String, String),
    #[error("Failed to resolve VRL expression for timeout for subgraph. Runtime error: {0}")]
    #[strum(serialize = "SUBGRAPH_TIMEOUT_EXPRESSION_RESOLUTION_FAILURE")]
    TimeoutExpressionResolution(String),
    #[error("Request to subgraph timed out after {0} milliseconds")]
    #[strum(serialize = "SUBGRAPH_REQUEST_TIMEOUT")]
    RequestTimeout(u128),
    #[error("Failed to deserialize subgraph response: {0}")]
    #[strum(serialize = "SUBGRAPH_RESPONSE_DESERIALIZATION_FAILURE")]
    ResponseDeserializationFailure(sonic_rs::Error),
    #[error("Failed to initialize or load native TLS root certificates: {0}")]
    #[strum(serialize = "SUBGRAPH_HTTPS_CERTS_FAILURE")]
    NativeTlsCertificatesError(std::io::Error),
}

impl SubgraphExecutorError {
    pub fn error_code(&self) -> &'static str {
        self.into()
    }
}
