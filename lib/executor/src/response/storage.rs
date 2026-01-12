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

    pub fn add_response(&mut self, response: Bytes) {
        self.responses.push(response);
    }

    pub fn estimate_final_response_size(&self) -> usize {
        let total_size: usize = self.responses.iter().map(|r| r.len()).sum();
        // Add a 20% buffer to account for JSON syntax, escaping, and other overhead.
        // I tested a bunch of numbers and it was the best from the bunch.
        (total_size as f64 * 1.2) as usize
    }
}
