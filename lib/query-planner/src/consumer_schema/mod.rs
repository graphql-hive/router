pub(crate) mod prune_inacessible;
pub(crate) mod strip_schema_internals;

use std::hash::{Hash, Hasher};

use graphql_tools::parser::schema::*;
use prune_inacessible::PruneInaccessible;
use strip_schema_internals::StripSchemaInternals;
use xxhash_rust::xxh3::Xxh3;

#[derive(Debug)]
pub struct ConsumerSchema {
    pub document: Document<'static, String>,
    pub hash: u64,
}

impl ConsumerSchema {
    pub fn new_from_supergraph(supergraph: &Document<'static, String>) -> Self {
        let document: Document<'static, String> = Self::create_consumer_schema(supergraph).into();
        let hash = {
            let mut hasher = Xxh3::new();
            document.to_string().hash(&mut hasher);
            hasher.finish()
        };
        Self { document, hash }
    }

    fn create_consumer_schema(supergraph: &Document<'static, String>) -> Document<'static, String> {
        let mut result = PruneInaccessible::prune(supergraph);
        result = StripSchemaInternals::strip_schema_internals(&result);
        // Add introspection schema to the consumer schema
        let introspection_schema = include_str!("introspection_schema.graphql");
        let mut parsed_introspection_schema =
            graphql_tools::parser::schema::parse_schema(introspection_schema).unwrap();
        parsed_introspection_schema
            .definitions
            .iter_mut()
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
                                    result.definitions.push(def.clone());
                                }
                            }
                        } else {
                            // Add other types from introspection schema
                            result.definitions.push(def.clone());
                        }
                    }
                    _ => result.definitions.push(def.clone()),
                }
            });
        result
    }
}
