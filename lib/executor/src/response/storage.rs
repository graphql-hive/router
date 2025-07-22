use simd_json::BorrowedValue;

use crate::response::{merge::deep_merge, value::Value};

pub struct ResponsesStorage<'req> {
    // arena: &'req Bump,
    responses: Vec<&'req BorrowedValue<'req>>,
    pub final_response: Value<'req>,
}

impl<'req> ResponsesStorage<'req> {
    pub fn new() -> Self {
        Self {
            // arena,
            responses: Vec::new(),
            final_response: Value::Null,
        }
    }

    pub fn add_response<'sub_req: 'req>(&mut self, response: &'sub_req BorrowedValue<'req>) {
        self.responses.push(response);
        deep_merge(&mut self.final_response, response);
    }
}
