#[derive(Debug, thiserror::Error)]
pub enum MinificationError {
    #[error("Type not found: {0}")]
    TypeNotFound(String),
    #[error("Field '{0}' not found in type '{1}'")]
    FieldNotFound(String, String),
    #[error("Unsupported fragment spread")]
    UnsupportedFragmentSpread,
    #[error("Unsupported field in `_entities`: {0}.{1}")]
    UnsupportedFieldInEntities(String, String),
}
