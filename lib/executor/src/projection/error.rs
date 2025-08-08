#[derive(thiserror::Error, Debug, Clone)]
pub enum ProjectionError {
    #[error("Failed to serialize GraphQL Errors: {0}")]
    ErrorsSerializationError(String),
    #[error("Failed to serialize GraphQL Extensions: {0}")]
    ExtensionsSerializationError(String),
    #[error("Failed to serialize a custom scalar: {0}")]
    CustomScalarSerializationError(String),
    #[error("Field named '{0}' was not found in definition name '{1}'")]
    FieldDefinitionNotFound(String, String),
}
