use simd_json::BorrowedValue;

use crate::{
    response::{merge::deep_merge, value::Value},
    ParsedResponse,
};

pub struct ResponsesStorage<'req> {
    // arena: &'req Bump,
    responses: Vec<ParsedResponse>,
    pub final_response: Value<'req>,
}

impl<'req> ResponsesStorage<'req> {
    pub fn new() -> Self {
        Self {
            responses: Vec::new(),
            final_response: Value::Null,
        }
    }

    pub fn add_response(&mut self, response: ParsedResponse) {
        let new_item_index = self.responses.len();
        self.responses.push(response);
        self.responses[new_item_index].with_json(|json| {
            let json_with_req_lifetime: &BorrowedValue<'req> = unsafe { std::mem::transmute(json) };
            deep_merge(&mut self.final_response, &json_with_req_lifetime);
        });
    }
}
