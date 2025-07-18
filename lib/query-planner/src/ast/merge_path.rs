use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display, Write},
    sync::Arc,
};

use crate::ast::selection_set::{FieldSelection, InlineFragmentSelection};

// Can be either the alias or the name of the field. This will be used to identify the field in the selection set.
type SelectionIdentifier = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    Skip(String),
    Include(String),
}

impl Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skip(condition) => write!(f, "@skip(if: ${})", condition),
            Self::Include(condition) => write!(f, "@include(if: ${})", condition),
        }
    }
}

impl From<&FieldSelection> for Option<Condition> {
    fn from(field: &FieldSelection) -> Self {
        if let Some(variable) = &field.skip_if {
            return Some(Condition::Skip(variable.clone()));
        }
        if let Some(variable) = &field.include_if {
            return Some(Condition::Include(variable.clone()));
        }
        None
    }
}

impl From<&mut FieldSelection> for Option<Condition> {
    fn from(field: &mut FieldSelection) -> Self {
        if let Some(variable) = &field.skip_if {
            return Some(Condition::Skip(variable.clone()));
        }
        if let Some(variable) = &field.include_if {
            return Some(Condition::Include(variable.clone()));
        }
        None
    }
}

impl From<&InlineFragmentSelection> for Option<Condition> {
    fn from(fragment: &InlineFragmentSelection) -> Self {
        if let Some(variable) = &fragment.skip_if {
            return Some(Condition::Skip(variable.clone()));
        }

        if let Some(variable) = &fragment.include_if {
            return Some(Condition::Include(variable.clone()));
        }

        None
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Segment {
    // A field with a unique identifier and the arguments hash
    // We used this to uniquely identify the field in the selection set.
    Field(SelectionIdentifier, u64, Option<Condition>),
    List,
    Cast(String, Option<Condition>),
}

impl Display for Segment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::List => write!(f, "@"),
            Self::Cast(type_name, condition) => {
                if let Some(condition) = condition {
                    write!(f, "|[{}] {}", type_name, condition)
                } else {
                    write!(f, "|[{}]", type_name)
                }
            }
            Self::Field(field_name, _, condition) => {
                if let Some(condition) = condition {
                    write!(f, "{} {}", field_name, condition)
                } else {
                    write!(f, "{}", field_name)
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)] // Clone is cheap with Arc inside
pub struct MergePath {
    pub inner: Arc<[Segment]>,
}

impl MergePath {
    pub fn new(path: Vec<Segment>) -> Self {
        Self { inner: path.into() }
    }

    pub fn slice_from(&self, start: usize) -> Self {
        Self {
            inner: Arc::from(&self.inner[start..]),
        }
    }

    pub fn last(&self) -> Option<&Segment> {
        self.inner.last()
    }

    pub fn without_last(&self) -> Self {
        Self {
            inner: Arc::from(&self.inner[..self.inner.len() - 1]),
        }
    }

    pub fn join(&self, sep: &str) -> String {
        if self.inner.is_empty() {
            return String::new();
        }

        let mut result = String::new();
        let mut iter = self.inner.iter();
        // .filter(|segment| !matches!(segment, Segment::Cast(_, _)));

        // We take the first to avoid a leading separator
        if let Some(first_segment) = iter.next() {
            write!(result, "{}", first_segment).unwrap();
        }

        for segment in iter {
            result.push_str(sep);
            write!(result, "{}", segment).unwrap();
        }

        result
    }

    /// Insert a string at the beginning of the path
    pub fn insert_front(&self, segment: impl Into<Segment>) -> Self {
        let mut new_segments = Vec::with_capacity(self.inner.len() + 1);
        new_segments.push(segment.into());
        new_segments.extend_from_slice(&self.inner);
        Self::new(new_segments)
    }

    /// Inserts a string at the end of the path
    pub fn push(&self, segment: impl Into<Segment>) -> Self {
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
            .take_while(|(s, o)| s == o)
            .count()
    }

    pub fn starts_with(&self, other: &MergePath) -> bool {
        if other.len() > self.len() {
            return false;
        }
        self.common_prefix_len(other) == other.len()
    }
}

impl Display for MergePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.inner.iter();
        // .filter(|segment| !matches!(segment, Segment::Cast(_, _)));

        // We take the first to avoid a leading separator
        if let Some(first_segment) = iter.next() {
            write!(f, "{}", first_segment).unwrap();
        }

        for segment in iter {
            write!(f, ".{}", segment).unwrap();
        }

        Ok(())
    }
}

impl From<MergePath> for Vec<String> {
    fn from(path: MergePath) -> Self {
        (&path).into()
    }
}

impl From<&MergePath> for Vec<String> {
    fn from(path: &MergePath) -> Self {
        path.inner
            .iter()
            // .filter(|segment| !matches!(segment, Segment::Cast(_, _)))
            .cloned()
            .map(|segment| format!("{}", segment))
            .collect()
    }
}
