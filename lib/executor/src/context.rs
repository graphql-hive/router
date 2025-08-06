use crate::response::{storage::ResponsesStorage, value::Value};

pub struct ExecutionContext<'a> {
    pub response_storage: ResponsesStorage,
    pub final_response: Value<'a>,
}

impl<'a> Default for ExecutionContext<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> ExecutionContext<'a> {
    pub fn new() -> Self {
        ExecutionContext {
            response_storage: ResponsesStorage::new(),
            final_response: Value::Null,
        }
    }
}
