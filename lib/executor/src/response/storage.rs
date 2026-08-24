use bytes::Bytes;

use crate::response::arena::ResponseArena;

/// Keeps alive everything the merged response tree borrows from: the subgraph response
/// buffers its strings point into, and the arenas its objects and lists were allocated in.
///
/// Nothing reads the arenas back; they are held so that dropping this storage — once the
/// response has been projected — is what frees the whole tree, in a handful of chunk frees
/// rather than one per node.
pub struct ResponsesStorage {
    responses: Vec<Bytes>,
    arenas: Vec<ResponseArena>,
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
            arenas: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.responses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.responses.is_empty()
    }

    pub fn add_response(&mut self, response: Bytes) {
        self.responses.push(response);
    }

    /// Takes ownership of the arena a response was parsed into, so values merged out of that
    /// response stay valid for the rest of the request.
    pub fn add_arena(&mut self, arena: ResponseArena) {
        self.arenas.push(arena);
    }

    pub fn estimate_final_response_size(&self) -> usize {
        let total_size: usize = self.responses.iter().map(|r| r.len()).sum();
        // Add a 20% buffer to account for JSON syntax, escaping, and other overhead.
        // I tested a bunch of numbers and it was the best from the bunch.
        (total_size as f64 * 1.2) as usize
    }
}
