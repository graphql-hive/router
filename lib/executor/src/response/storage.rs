use std::sync::Arc;

use bytes::Bytes;

pub struct ResponsesStorage {
    responses: Vec<Arc<Bytes>>,
}

impl Default for ResponsesStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponsesStorage {
    pub fn new() -> Self {
        Self {
            responses: Vec::new(),
        }
    }

    pub fn add_response(&mut self, response: Bytes) -> usize {
        let new_item_index = self.responses.len();
        self.responses.push(Arc::new(response));
        new_item_index
    }

    // This helper is what we need
    pub fn get_bytes(&self, index: usize) -> &[u8] {
        &self.responses[index]
    }
}
