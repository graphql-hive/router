use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::{Debug, Display, Write};
use std::rc::Rc;

use crate::query_planner::ast::selection_set::{FieldSelection, InlineFragmentSelection};

// This is used to identify the field in the selection set
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FieldPathSegment {
    pub field_name: String,
    pub alias: Option<String>,
}

impl FieldPathSegment {
    pub fn new(field_name: String, alias: Option<String>) -> Self {
        Self { field_name, alias }
    }

    pub fn named(field_name: String) -> Self {
        Self {
            field_name,
            alias: None,
        }
    }

    pub fn response_key(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.field_name)
    }

    pub fn field_name(&self) -> &str {
        &self.field_name
    }
}

impl Display for FieldPathSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.response_key())
    }
}

impl From<&FieldSelection> for FieldPathSegment {
    fn from(field: &FieldSelection) -> Self {
        Self {
            field_name: field.name.clone(),
            alias: field.alias.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Condition {
    Skip(String),
    Include(String),
    SkipAndInclude { skip: String, include: String },
}

impl Condition {
    pub fn to_skip_if(&self) -> Option<String> {
        match self {
            Condition::Skip(var) => Some(var.clone()),
            Condition::SkipAndInclude { skip, .. } => Some(skip.clone()),
            _ => None,
        }
    }
    pub fn to_include_if(&self) -> Option<String> {
        match self {
            Condition::Include(var) => Some(var.clone()),
            Condition::SkipAndInclude { include, .. } => Some(include.clone()),
            _ => None,
        }
    }
}

impl Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skip(condition) => write!(f, "@skip(if: ${})", condition),
            Self::Include(condition) => write!(f, "@include(if: ${})", condition),
            Self::SkipAndInclude { skip, include } => {
                write!(f, "@skip(if: ${}) @include(if: ${})", skip, include)
            }
        }
    }
}

impl From<&FieldSelection> for Option<Condition> {
    fn from(field: &FieldSelection) -> Self {
        match (&field.skip_if, &field.include_if) {
            (Some(skip), Some(include)) => Some(Condition::SkipAndInclude {
                skip: skip.clone(),
                include: include.clone(),
            }),
            (Some(variable), None) => Some(Condition::Skip(variable.clone())),
            (None, Some(variable)) => Some(Condition::Include(variable.clone())),
            (None, None) => None,
        }
    }
}

impl From<&mut FieldSelection> for Option<Condition> {
    fn from(field: &mut FieldSelection) -> Self {
        match (&field.skip_if, &field.include_if) {
            (Some(skip), Some(include)) => Some(Condition::SkipAndInclude {
                skip: skip.clone(),
                include: include.clone(),
            }),
            (Some(variable), None) => Some(Condition::Skip(variable.clone())),
            (None, Some(variable)) => Some(Condition::Include(variable.clone())),
            (None, None) => None,
        }
    }
}

impl From<&InlineFragmentSelection> for Option<Condition> {
    fn from(fragment: &InlineFragmentSelection) -> Self {
        match (&fragment.skip_if, &fragment.include_if) {
            (Some(skip), Some(include)) => Some(Condition::SkipAndInclude {
                skip: skip.clone(),
                include: include.clone(),
            }),
            (Some(variable), None) => Some(Condition::Skip(variable.clone())),
            (None, Some(variable)) => Some(Condition::Include(variable.clone())),
            (None, None) => None,
        }
    }
}

/// Cheap to clone: paths get copied on every push, so the strings sit behind `Rc`.
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    // A field with a unique identifier and the arguments hash
    // We used this to uniquely identify the field in the selection set.
    Field(Rc<FieldPathSegment>, u64, Option<Condition>),
    List,
    TypeCondition(Rc<BTreeSet<String>>, Option<Condition>),
}

impl Display for Segment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::List => write!(f, "@"),
            Self::TypeCondition(type_names, condition) => {
                let joined = type_names.iter().cloned().collect::<Vec<_>>().join("|");
                if let Some(condition) = condition {
                    write!(f, "|[{}] {}", joined, condition)
                } else {
                    write!(f, "|[{}]", joined)
                }
            }
            Self::Field(field_seg, _, condition) => {
                if let Some(condition) = condition {
                    write!(f, "{} {}", field_seg.response_key(), condition)
                } else {
                    write!(f, "{}", field_seg.response_key())
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MergePath {
    pub inner: Rc<[Segment]>,
}

impl MergePath {
    pub fn new(path: Vec<Segment>) -> Self {
        Self { inner: path.into() }
    }

    pub fn slice_from(&self, start: usize) -> Self {
        Self {
            inner: self.inner[start..].iter().cloned().collect(),
        }
    }

    pub fn last(&self) -> Option<&Segment> {
        self.inner.last()
    }

    pub fn without_last(&self) -> Self {
        Self {
            inner: self.inner[..self.inner.len() - 1].iter().cloned().collect(),
        }
    }

    pub fn join(&self, sep: &str) -> String {
        if self.inner.is_empty() {
            return String::new();
        }

        let mut result = String::new();
        let mut iter = self.inner.iter();
        // .filter(|segment| !matches!(segment, Segment::TypeCondition(_, _)));

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

    // These collect straight into the `Rc<[Segment]>`. The iterators know their exact length,
    // so that's one allocation, no Vec in between.

    /// Insert a string at the beginning of the path
    pub fn insert_front(&self, segment: impl Into<Segment>) -> Self {
        Self {
            inner: std::iter::once(segment.into())
                .chain(self.inner.iter().cloned())
                .collect(),
        }
    }

    /// Inserts a string at the end of the path
    pub fn push(&self, segment: impl Into<Segment>) -> Self {
        Self {
            inner: self
                .inner
                .iter()
                .cloned()
                .chain(std::iter::once(segment.into()))
                .collect(),
        }
    }

    /// Appends another path to this one
    pub fn concat(&self, other: &MergePath) -> Self {
        Self {
            inner: self
                .inner
                .iter()
                .chain(other.inner.iter())
                .cloned()
                .collect(),
        }
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

    /// The conditions along the path, outermost first.
    pub fn conditions(&self) -> impl Iterator<Item = &Condition> {
        self.inner.iter().filter_map(|segment| match segment {
            Segment::TypeCondition(_, Some(condition)) | Segment::Field(_, _, Some(condition)) => {
                Some(condition)
            }
            _ => None,
        })
    }

    /// Whether the two paths are the same once their type conditions are taken out.
    pub fn eq_without_type_castings(&self, other: &Self) -> bool {
        let not_type_condition =
            |segment: &&Segment| !matches!(segment, Segment::TypeCondition(..));
        self.inner
            .iter()
            .filter(not_type_condition)
            .eq(other.inner.iter().filter(not_type_condition))
    }
}

impl Display for MergePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.inner.iter();
        // .filter(|segment| !matches!(segment, Segment::TypeCondition(_, _)));

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
            // .filter(|segment| !matches!(segment, Segment::TypeCondition(_, _)))
            .map(|segment| format!("{}", segment))
            .collect()
    }
}
