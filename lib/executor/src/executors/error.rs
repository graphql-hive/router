use hive_router_internal::expressions::vrl::prelude::ExpressionError;

use crate::response::graphql_error::GraphQLError;

#[derive(thiserror::Error, Debug, Clone)]
pub enum SubgraphExecutorError {
    #[error("Failed to parse endpoint \"{0}\" as URI: {1}")]
    EndpointParseFailure(String, String),
    #[error("Failed to compile VRL expression for subgraph '{0}'. Please check your VRL expression for syntax errors. Diagnostic: {1}")]
    EndpointExpressionBuild(String, String),
    #[error("Failed to resolve VRL expression for subgraph '{0}'. Runtime error: {1}")]
    EndpointExpressionResolutionFailure(String, String),
    #[error("VRL expression for subgraph '{0}' resolved to a non-string value.")]
    EndpointExpressionWrongType(String),
    #[error("Static endpoint not found for subgraph '{0}'. This is an internal error and should not happen.")]
    StaticEndpointNotFound(String),
    #[error("Failed to build request to subgraph \"{0}\": {1}")]
    RequestBuildFailure(String, String),
    #[error("Failed to send request to subgraph \"{0}\": {1}")]
    RequestFailure(String, String),
    #[error("Failed to serialize variable \"{0}\": {1}")]
    VariablesSerializationFailure(String, String),
    #[error("Failed to compile VRL expression for timeout for subgraph '{0}'. Please check your VRL expression for syntax errors. Diagnostic: {1}")]
    RequestTimeoutExpressionBuild(String, String),
    #[error("Failed to resolve VRL expression for timeout for subgraph '{0}'. Runtime error: {1}")]
    TimeoutExpressionResolution(String, String),
    #[error("Request to subgraph \"{0}\" timed out after {1} milliseconds")]
    RequestTimeout(String, u128),
    #[error("Failed to deserialize subgraph response: {0}")]
    ResponseDeserializationFailure(String),
}

impl From<SubgraphExecutorError> for GraphQLError {
    fn from(error: SubgraphExecutorError) -> Self {
        GraphQLError::from_message_and_code("Internal server error", error.error_code())
    }
}

impl SubgraphExecutorError {
    pub fn new_endpoint_expression_resolution_failure(
        subgraph_name: String,
        error: ExpressionError,
    ) -> Self {
        SubgraphExecutorError::EndpointExpressionResolutionFailure(subgraph_name, error.to_string())
    }

    pub fn error_code(&self) -> &'static str {
        match self {
            SubgraphExecutorError::EndpointParseFailure(_, _) => "SUBGRAPH_ENDPOINT_PARSE_FAILURE",
            SubgraphExecutorError::EndpointExpressionBuild(_, _) => {
                "SUBGRAPH_ENDPOINT_EXPRESSION_BUILD_FAILURE"
            }
            SubgraphExecutorError::EndpointExpressionResolutionFailure(_, _) => {
                "SUBGRAPH_ENDPOINT_EXPRESSION_RESOLUTION_FAILURE"
            }
            SubgraphExecutorError::EndpointExpressionWrongType(_) => {
                "SUBGRAPH_ENDPOINT_EXPRESSION_WRONG_TYPE"
            }
            SubgraphExecutorError::StaticEndpointNotFound(_) => {
                "SUBGRAPH_STATIC_ENDPOINT_NOT_FOUND"
            }
            SubgraphExecutorError::RequestBuildFailure(_, _) => "SUBGRAPH_REQUEST_BUILD_FAILURE",
            SubgraphExecutorError::RequestFailure(_, _) => "SUBGRAPH_REQUEST_FAILURE",
            SubgraphExecutorError::VariablesSerializationFailure(_, _) => {
                "SUBGRAPH_VARIABLES_SERIALIZATION_FAILURE"
            }
            SubgraphExecutorError::TimeoutExpressionResolution(_, _) => {
                "SUBGRAPH_TIMEOUT_EXPRESSION_RESOLUTION_FAILURE"
            }
            SubgraphExecutorError::RequestTimeout(_, _) => "SUBGRAPH_REQUEST_TIMEOUT",
            SubgraphExecutorError::RequestTimeoutExpressionBuild(_, _) => {
                "SUBGRAPH_TIMEOUT_EXPRESSION_BUILD_FAILURE"
            }
            SubgraphExecutorError::ResponseDeserializationFailure(_) => {
                "SUBGRAPH_RESPONSE_DESERIALIZATION_FAILURE"
            }
        }
    }
}
