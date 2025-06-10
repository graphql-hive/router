pub mod introspection;
pub(crate) mod prune_inacessible;
pub mod schema_metadata;
pub(crate) mod strip_schema_internals;
pub mod value_from_ast;

use graphql_parser::schema::*;
use prune_inacessible::PruneInaccessible;
use strip_schema_internals::StripSchemaInternals;

use crate::consumer_schema::schema_metadata::SchemaMetadata;

static INTROSPECTION_SCHEMA: &str = include_str!("introspection_schema.graphql");

#[derive(Debug, Clone)]
pub struct ConsumerSchema {
    pub document: Document<'static, String>,
    pub schema_metadata: SchemaMetadata,
}

impl ConsumerSchema {
    pub fn new_from_supergraph(supergraph: &Document<'static, String>) -> Self {
        let document = Self::create_consumer_schema(supergraph);
        let schema_metadata = schema_metadata::create_schema_metadata(&document);
        Self {
            document,
            schema_metadata,
        }
    }

    fn create_consumer_schema(supergraph: &Document<'static, String>) -> Document<'static, String> {
        let mut result = PruneInaccessible::prune(supergraph);
        result = StripSchemaInternals::strip_schema_internals(&result);
        // Add introspection schema to the consumer schema
        graphql_parser::schema::parse_schema(INTROSPECTION_SCHEMA)
            .unwrap()
            .definitions
            .into_iter()
            .for_each(|def| {
                match def {
                    Definition::TypeDefinition(TypeDefinition::Object(
                        type_def_in_introspection,
                    )) => {
                        if type_def_in_introspection.name == "Query" {
                            match result.definitions.iter_mut().find(|d| {
                                if let Definition::TypeDefinition(TypeDefinition::Object(
                                    query_def,
                                )) = d
                                {
                                    query_def.name == "Query"
                                } else {
                                    false
                                }
                            }) {
                                Some(Definition::TypeDefinition(TypeDefinition::Object(
                                    query_def,
                                ))) => {
                                    // Query type already exists, extend it
                                    query_def
                                        .fields
                                        .extend(type_def_in_introspection.fields.clone());
                                }
                                _ => {
                                    // Add the Query type from introspection schema
                                    result.definitions.push(Definition::TypeDefinition(
                                        TypeDefinition::Object(type_def_in_introspection),
                                    ));
                                }
                            }
                        } else {
                            // Add other types from introspection schema
                            result.definitions.push(Definition::TypeDefinition(
                                TypeDefinition::Object(type_def_in_introspection),
                            ));
                        }
                    }
                    _ => result.definitions.push(def),
                }
            });
        result
    }
}
