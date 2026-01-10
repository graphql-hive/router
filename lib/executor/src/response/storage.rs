use bytes::Bytes;

pub struct ResponsesStorage {
    responses: Vec<Bytes>,
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
        self.responses.push(response);
        new_item_index
    }

    pub fn estimate_final_response_size(&self) -> usize {
        self.responses.iter().map(|r| r.len()).sum()
    }
}
