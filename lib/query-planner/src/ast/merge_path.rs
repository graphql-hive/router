use std::{fmt::Debug, sync::Arc};

#[derive(Clone, Debug, Default)] // Clone is cheap with Arc inside
pub struct MergePath {
    pub inner: Arc<[String]>,
}

impl MergePath {
    pub fn new(path: Vec<String>) -> Self {
        Self { inner: path.into() }
    }

    pub fn slice_from(&self, start: usize) -> Self {
        Self {
            inner: Arc::from(&self.inner[start..]),
        }
    }

    pub fn join(&self, sep: &str) -> String {
        self.inner.join(sep)
    }

    /// Insert a string at the beginning of the path
    pub fn insert_front(&self, segment: impl Into<String>) -> Self {
        let mut new_segments = Vec::with_capacity(self.inner.len() + 1);
        new_segments.push(segment.into());
        new_segments.extend_from_slice(&self.inner);
        Self::new(new_segments)
    }

    /// Inserts a string at the end of the path
    pub fn push(&self, segment: impl Into<String>) -> Self {
        let mut new_segments = Vec::with_capacity(self.inner.len() + 1);
        new_segments.extend_from_slice(&self.inner);
        new_segments.push(segment.into());
        Self::new(new_segments)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn common_prefix_len(&self, other: &MergePath) -> usize {
        self.inner
            .iter()
            .zip(other.inner.iter())
            .take_while(|(a, b)| a == b)
            .count()
    }

    pub fn starts_with(&self, other: &MergePath) -> bool {
        if other.len() > self.len() {
            return false;
        }
        self.common_prefix_len(other) == other.len()
    }
}

impl PartialEq for MergePath {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}
