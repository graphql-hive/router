use graphql_parser_hive_fork::schema::*;

use super::{prune_inacessible::PruneInaccessible, strip_schema_internals::StripSchemaInternals};

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
        let mut result = PruneInaccessible::prune(&supergraph);
        result = StripSchemaInternals::strip_schema_internals(&result);

        result
    }
}
