#[derive(thiserror::Error, Debug, Clone)]
pub enum SubgraphExecutorError {
    #[error("Failed to parse endpoint \"{0}\" as URI: {1}")]
    EndpointParseFailure(String, String),
    #[error("Failed to build request to subgraph \"{0}\": {1}")]
    RequestBuildFailure(String, String),
    #[error("Failed to send request to subgraph \"{0}\": {1}")]
    RequestFailure(String, String),
    #[error("Failed to serialize variable \"{0}\": {1}")]
    VariablesSerializationFailure(String, String),
}
