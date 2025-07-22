use crate::{response::storage::ResponsesStorage, schema::metadata::SchemaMetadata};

pub struct ExecutionContext<'a> {
    pub response_storage: ResponsesStorage<'a>,
    pub schema_metadata: &'a SchemaMetadata<'a>,
}

impl<'a> ExecutionContext<'a> {
    pub fn new(schema_metadata: &'a SchemaMetadata<'a>) -> Self {
        ExecutionContext {
            response_storage: ResponsesStorage::new(),
            schema_metadata,
        }
    }
}
