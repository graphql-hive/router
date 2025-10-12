use std::time::Duration;

use bytes::{BufMut, Bytes, BytesMut};

use crate::response::graphql_error::GraphQLError;

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
    #[error("Failed to parse timeout duration from expression: {0}")]
    TimeoutExpressionParseFailure(String),
    #[error("Request timed out after {0:?}")]
    RequestTimeout(Duration),
}
pub fn error_to_graphql_bytes(endpoint: &http::Uri, e: SubgraphExecutorError) -> Bytes {
    let graphql_error: GraphQLError =
        format!("Failed to execute request to subgraph {}: {}", endpoint, e).into();
    let errors = vec![graphql_error];
    // This unwrap is safe as GraphQLError serialization shouldn't fail.
    let errors_bytes = sonic_rs::to_vec(&errors).unwrap();
    let mut buffer = BytesMut::new();
    buffer.put_slice(b"{\"errors\":");
    buffer.put_slice(&errors_bytes);
    buffer.put_slice(b"}");
    buffer.freeze()
}
