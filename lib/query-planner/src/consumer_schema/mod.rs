pub(crate) mod prune_inacessible;
pub(crate) mod strip_schema_internals;

use graphql_parser::schema::*;
use prune_inacessible::PruneInaccessible;
use strip_schema_internals::StripSchemaInternals;

#[derive(Debug, Clone)]
pub struct ConsumerSchema {
    pub document: Document<'static, String>,
}

impl ConsumerSchema {
    pub fn new_from_supergraph(supergraph: &Document<'static, String>) -> Self {
        Self {
            document: Self::create_consumer_schema(supergraph),
        }
    }

    fn create_consumer_schema(supergraph: &Document<'static, String>) -> Document<'static, String> {
        let mut result = PruneInaccessible::prune(supergraph);
        result = StripSchemaInternals::strip_schema_internals(&result);
        let introspection_schema = include_str!("introspection_schema.graphql");
        let parsed_introspection_schema =
            graphql_parser::schema::parse_schema(introspection_schema).unwrap();
        parsed_introspection_schema
            .definitions
            .iter()
            .for_each(|def| result.definitions.push(def.clone()));
        result
    }
}
