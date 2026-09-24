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

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Segment {
    // A field with a unique identifier and the arguments hash
    // We used this to uniquely identify the field in the selection set.
    Field(FieldPathSegment, u64, Option<Condition>),
    List,
    TypeCondition(BTreeSet<String>, Option<Condition>),
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
            inner: Rc::from(&self.inner[start..]),
        }
    }

    pub fn last(&self) -> Option<&Segment> {
        self.inner.last()
    }

    pub fn without_last(&self) -> Self {
        Self {
            inner: Rc::from(&self.inner[..self.inner.len() - 1]),
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

    /// Inserts a string at the end of the path
    pub fn push(&self, segment: impl Into<Segment>) -> Self {
        let mut new_segments = Vec::with_capacity(self.inner.len() + 1);
        new_segments.extend_from_slice(&self.inner);
        new_segments.push(segment.into());
        Self::new(new_segments)
    }

    /// Appends another path to this one
    pub fn concat(&self, other: &MergePath) -> Self {
        let mut new_segments = Vec::with_capacity(self.inner.len() + other.inner.len());
        new_segments.extend_from_slice(&self.inner);
        new_segments.extend_from_slice(&other.inner);
        Self::new(new_segments)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Where `prefix` sits at the start of this path, for places in the response, where two
    /// paths can point at the same objects without being written the same way.
    ///
    /// Fields match by response key and arguments, lists match lists, and `@skip`/`@include`
    /// on segments don't count. Type conditions only narrow which objects a path points at, so
    /// type conditions on one side alone are fine. When both sides have some at the same spot,
    /// `types_overlap` decides if they can be the same objects. It gets the type conditions of
    /// each side at that spot, outermost first, like `[{Node}, {Cat}]` for `|[Node]|[Cat]`.
    ///
    /// Returns the index in `self` where the rest starts. The rest keeps its type conditions.
    pub fn strip_location_prefix(
        &self,
        prefix: &MergePath,
        types_overlap: impl Fn(&[&BTreeSet<String>], &[&BTreeSet<String>]) -> bool,
    ) -> Option<usize> {
        // Type conditions from `path[idx..]`, and the index after them.
        fn types_at(path: &[Segment], mut idx: usize) -> (Vec<&BTreeSet<String>>, usize) {
            let mut types = Vec::new();
            while let Some(Segment::TypeCondition(names, _)) = path.get(idx) {
                types.push(names);
                idx += 1;
            }
            (types, idx)
        }
        let overlap = |a: &[&BTreeSet<String>], b: &[&BTreeSet<String>]| {
            a.is_empty() || b.is_empty() || types_overlap(a, b)
        };

        let (path, prefix) = (&self.inner[..], &prefix.inner[..]);
        let (mut i, mut j) = (0, 0);
        loop {
            let (path_types, next_i) = types_at(path, i);
            let (prefix_types, next_j) = types_at(prefix, j);
            if !overlap(&path_types, &prefix_types) {
                return None;
            }
            if next_j == prefix.len() {
                // The rest starts with our own type conditions, if we have any.
                return Some(i);
            }
            let same = match (path.get(next_i), &prefix[next_j]) {
                (Some(Segment::List), Segment::List) => true,
                (Some(Segment::Field(a, a_args, _)), Segment::Field(b, b_args, _)) => {
                    a.response_key() == b.response_key() && a_args == b_args
                }
                _ => false,
            };
            if !same {
                return None;
            }
            (i, j) = (next_i + 1, next_j + 1);
        }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{FieldPathSegment, MergePath, Segment};

    /// `a.@|[Cat|Dog].b` style: `@` is a list, `|[..]` a type condition, the rest fields.
    fn path(input: &str) -> MergePath {
        MergePath::new(
            input
                .split('.')
                .filter(|s| !s.is_empty())
                .flat_map(|part| {
                    let (field, types) = match part.split_once("|[") {
                        Some((field, types)) => (field, Some(types.trim_end_matches(']'))),
                        None => (part, None),
                    };
                    let field = match field {
                        "" => None,
                        "@" => Some(Segment::List),
                        name => Some(Segment::Field(
                            FieldPathSegment::named(name.to_string()),
                            0,
                            None,
                        )),
                    };
                    let types = types.map(|types| {
                        Segment::TypeCondition(
                            types
                                .split('|')
                                .map(str::to_string)
                                .collect::<BTreeSet<_>>(),
                            None,
                        )
                    });
                    field.into_iter().chain(types)
                })
                .collect(),
        )
    }

    #[test]
    fn strip_location_prefix_compares_types_only_on_both_sides() {
        // By name: any type in common.
        let names_overlap = |a: &[&BTreeSet<String>], b: &[&BTreeSet<String>]| {
            a.iter()
                .flat_map(|names| names.iter())
                .any(|name| b.iter().any(|names| names.contains(name)))
        };
        let rest = |p: &str, prefix: &str| {
            let p = path(p);
            p.strip_location_prefix(&path(prefix), names_overlap)
                .map(|idx| p.slice_from(idx).to_string())
        };

        assert_eq!(rest("a.@.b", "a.@"), Some("b".to_string()));
        assert_eq!(rest("a.@.b", "a.@.b"), Some("".to_string()));
        // A type condition on one side only.
        assert_eq!(rest("a.@|[Cat].b", "a.@.b"), Some("".to_string()));
        assert_eq!(rest("a.@.b.c", "a.@|[Cat].b"), Some("c".to_string()));
        assert_eq!(rest("a.@|[Cat].b", "a.@"), Some("|[Cat].b".to_string()));
        // On both sides.
        assert_eq!(rest("a.@|[Cat].b", "a.@|[Cat|Dog].b"), Some("".to_string()));
        assert_eq!(rest("a.@|[Cat].b", "a.@|[Dog].b"), None);
        assert_eq!(rest("a.@|[Cat].b", "a.@|[Dog]"), None);
        // Different fields, or a prefix that's longer.
        assert_eq!(rest("a.@.b", "a.@.c"), None);
        assert_eq!(rest("a.@", "a.@.b"), None);
        assert_eq!(rest("a.b", "a.@"), None);
    }
}
