use strum::IntoStaticStr;

#[derive(thiserror::Error, Debug, Clone, IntoStaticStr)]
pub enum SubgraphExecutorError {
    #[error("Failed to parse endpoint \"{0}\" as URI: {1}")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_PARSE_FAILURE")]
    EndpointParseFailure(String, String),
    #[error("Failed to compile VRL expression. Please check your VRL expression for syntax errors. Diagnostic: {0}")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_EXPRESSION_BUILD_FAILURE")]
    EndpointExpressionBuild(String),
    #[error("Failed to resolve VRL expression. Runtime error: {0}")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_EXPRESSION_RESOLUTION_FAILURE")]
    EndpointExpressionResolutionFailure(String),
    #[error("VRL expression resolved to a non-string value.")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_EXPRESSION_WRONG_TYPE")]
    EndpointExpressionWrongType(String),
    #[error("Static endpoint not found. This is an internal error and should not happen.")]
    #[strum(serialize = "SUBGRAPH_STATIC_ENDPOINT_NOT_FOUND")]
    StaticEndpointNotFound,
    #[error("Failed to build request: {0}")]
    #[strum(serialize = "SUBGRAPH_REQUEST_BUILD_FAILURE")]
    RequestBuildFailure(String),
    #[error("Failed to send request: {0}")]
    #[strum(serialize = "SUBGRAPH_REQUEST_FAILURE")]
    RequestFailure(String),
    #[error("Failed to receive response: {0}")]
    #[strum(serialize = "SUBGRAPH_RESPONSE_FAILURE")]
    ResponseFailure(String),
    #[error("Failed to serialize variable \"{0}\": {1}")]
    #[strum(serialize = "SUBGRAPH_VARIABLES_SERIALIZATION_FAILURE")]
    VariablesSerializationFailure(String, String),
    #[error("Failed to compile VRL expression for timeout for subgraph '{0}'. Please check your VRL expression for syntax errors. Diagnostic: {1}")]
    #[strum(serialize = "SUBGRAPH_TIMEOUT_EXPRESSION_BUILD_FAILURE")]
    RequestTimeoutExpressionBuild(String, String),
    #[error("Failed to resolve VRL expression for timeout for subgraph '{0}'. Runtime error: {1}")]
    #[strum(serialize = "SUBGRAPH_TIMEOUT_EXPRESSION_RESOLUTION_FAILURE")]
    TimeoutExpressionResolution(String, String),
    #[error("Request to subgraph timed out after {0} milliseconds")]
    #[strum(serialize = "SUBGRAPH_REQUEST_TIMEOUT")]
    RequestTimeout(u128),
    #[error("Failed to deserialize subgraph response: {0}")]
    #[strum(serialize = "SUBGRAPH_RESPONSE_DESERIALIZATION_FAILURE")]
    ResponseDeserializationFailure(String),
}
