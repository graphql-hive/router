use std::sync::Arc;

use bytes::Bytes;

pub struct ResponsesStorage {
    responses: Vec<Arc<Bytes>>,
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
    pub fn get_bytes<'a>(&'a self, index: usize) -> &'a [u8] {
        &self.responses[index]
    }

    pub fn add_responses(&mut self, responses: Vec<Bytes>) -> Vec<usize> {
        let start_index = self.responses.len();

        self.responses.reserve(responses.len());
        for res in responses {
            self.responses.push(Arc::new(res));
        }

        let mut list = Vec::with_capacity(start_index);

        for i in start_index..self.responses.len() {
            list.push(i);
        }

        list
    }

    // pub fn get_value_ref(&'a self, index: usize) -> ValueRef<'a> {
    //     self.responses[index].value.as_ref()
    // }

    // pub fn get_value_refs(&'a self, indices: &[usize]) -> Vec<ValueRef<'a>> {
    //     indices.iter().map(|&i| self.get_value_ref(i)).collect()
    // }
}
