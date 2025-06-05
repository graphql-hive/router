#[derive(Debug, thiserror::Error)]
pub enum NormalizationError {
    #[error("Expected a transformed operation, but found none.")]
    ExpectedTransformedOperationNotFound,

    #[error("Multiple operations found matching the criteria.")]
    MultipleMatchingOperationsFound,

    #[error("Specified operation '{operation_name}' not found.")]
    SpecifiedOperationNotFound { operation_name: String },

    #[error(
        "An anonymous operation was expected, but either none were present, or all were named."
    )]
    AnonymousOperationNotFound,

    #[error("Schema type '{type_name}' not found.")]
    SchemaTypeNotFound { type_name: String },

    #[error("Field '{field_name}' not found in type '{type_name}'.")]
    FieldNotFoundInType {
        field_name: String,
        type_name: String,
    },

    #[error("Possible types for '{type_name}' not found.")]
    PossibleTypesNotFound { type_name: String },

    #[error("Fragment definition for '{fragment_name}' not found.")]
    FragmentDefinitionNotFound { fragment_name: String },
}
