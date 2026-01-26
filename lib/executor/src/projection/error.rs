#[derive(thiserror::Error, Debug, Clone)]
pub enum ProjectionError {
    #[error("Failed to serialize GraphQL Errors: {0}")]
    ErrorsSerializationFailure(String),
    #[error("Failed to serialize GraphQL Extensions: {0}")]
    ExtensionsSerializationFailure(String),
    #[error("Type '{0}' not found in schema")]
    MissingType(String),
    #[error("Field '{field_name}' not found on type '{type_name}' in schema")]
    MissingField {
        field_name: String,
        type_name: String,
    },
}
