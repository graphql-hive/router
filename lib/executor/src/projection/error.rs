#[derive(thiserror::Error, Debug, Clone)]
pub enum ProjectionError {
    #[error("Failed to serialize GraphQL Errors: {0}")]
    ErrorsSerializationFailure(String),
    #[error("Failed to serialize GraphQL Extensions: {0}")]
    ExtensionsSerializationFailure(String),
    #[error("Failed to serialize a custom scalar: {0}")]
    CustomScalarSerializationFailure(String),
}
