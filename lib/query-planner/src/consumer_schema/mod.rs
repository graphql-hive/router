pub(crate) mod prune_inacessible;
pub(crate) mod strip_schema_internals;

use std::collections::{HashMap, HashSet};

use graphql_parser::schema::*;
use prune_inacessible::PruneInaccessible;
use strip_schema_internals::StripSchemaInternals;

type PossibleTypesMap = HashMap<String, HashSet<String>>;

#[derive(Debug, Clone)]
pub struct ConsumerSchema {
    pub document: Document<'static, String>,
    /// A collection of abstract types (union, interface) and their object types
    possible_types: PossibleTypesMap,
}

impl ConsumerSchema {
    pub fn new_from_supergraph(supergraph: &Document<'static, String>) -> Self {
        let document = Self::create_consumer_schema(supergraph);
        let possible_types = Self::possible_types_from_schema(&document);
        Self {
            document,
            possible_types,
        }
    }

    pub fn possible_types_of(&self, abstract_type_name: &str) -> Option<&HashSet<String>> {
        self.possible_types.get(abstract_type_name)
    }

    fn create_consumer_schema(supergraph: &Document<'static, String>) -> Document<'static, String> {
        let mut result = PruneInaccessible::prune(supergraph);
        result = StripSchemaInternals::strip_schema_internals(&result);
        // Add introspection schema to the consumer schema
        let introspection_schema = include_str!("introspection_schema.graphql");
        let mut parsed_introspection_schema =
            graphql_parser::schema::parse_schema(introspection_schema).unwrap();
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

    fn possible_types_from_schema(schema: &Document<'static, String>) -> PossibleTypesMap {
        let mut possible_types = PossibleTypesMap::new();

        for def in schema.definitions.iter() {
            if let Definition::TypeDefinition(type_def) = def {
                match type_def {
                    TypeDefinition::Union(union_def) => {
                        // Union type always points to object types,
                        // that's why we insert it directly
                        possible_types
                            .insert(union_def.name.clone(), vec_to_hashset(&union_def.types));
                    }
                    TypeDefinition::Object(object_def) => {
                        for interface_name in object_def.implements_interfaces.iter() {
                            possible_types
                                .entry(interface_name.to_string())
                                .and_modify(|entry| {
                                    entry.insert(object_def.name.clone());
                                })
                                .or_insert_with(|| vec_to_hashset(&[object_def.name.clone()]));
                        }
                    }
                    _ => {}
                }
            }
        }

        possible_types
    }
}

fn vec_to_hashset(values: &[String]) -> HashSet<String> {
    let mut hset: HashSet<String> = HashSet::new();

    for value in values {
        hset.insert(value.clone());
    }

    hset
}
