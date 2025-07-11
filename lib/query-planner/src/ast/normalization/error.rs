#[derive(Debug, thiserror::Error)]
pub enum NormalizationError {
    #[error("Expected a transformed operation, but found none.")]
    ExpectedTransformedOperationNotFound,

    #[error("Multiple operations found matching the criteria.")]
    MultipleMatchingOperationsFound,

    #[error("Specified operation '{operation_name}' not found.")]
    SpecifiedOperationNotFound { operation_name: String },

    #[error("An operation was expected, but none were present.")]
    OperationNotFound,

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

    /// In case of a shareable field resolving an interface, all object types implementing the interface
    /// must resolve the field in the same way.
    ///
    /// If one of the fields (defined by the interface) is @external in one of the object types,
    /// it means that the Query Planner would have to decide which subgraph to pick from to resolve the field
    /// of each individual object type.
    /// This would result in more than one request being made to the subgraphs.
    ///
    /// See: https://github.com/graphql-hive/federation-gateway-audit/blob/514fec87122d561a4f7b12a66a91a6a35b1a76a7/src/test-suites/corrupted-supergraph-node-id/test.ts#L6-L11
    #[error("The shareable field '{field_name}' on interface '{type_name}' is not resolvable by all of its object types in all subgraphs, which violates the '@shareable' contract.")]
    InconsistentShareableField {
        field_name: String,
        type_name: String,
    },
}
