use std::collections::BTreeSet;
use std::fmt::Display;

use serde::{Deserialize, Deserializer, Serialize};

use crate::query_planner::ast::merge_path::{MergePath, Segment};

use super::FetchNodePathSegment;

/// Uses the tagged format that `lib/node-addon` and `hive-expose-query-plan` expect:
/// `{"Field": name}`, `{"TypeCondition": [..]}`, and a plain `"@"` for a list step.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PathSegment {
    Field(Box<str>),
    /// Boxing this keeps every path step small.
    TypeCondition(Box<TypeCondition>),
    #[serde(rename = "@")]
    List,
}

/// The type names an entity may match. Sorted, with duplicates removed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct TypeCondition {
    names: Box<[Box<str>]>,
}

impl TypeCondition {
    /// Sorts the names and removes duplicates, so that two sets holding the same names
    /// compare, hash, and serialize the same way.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let unique: BTreeSet<&str> = names.into_iter().collect();
        Self {
            names: unique.into_iter().map(Box::<str>::from).collect(),
        }
    }

    pub fn names(&self) -> &[Box<str>] {
        &self.names
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FlattenNodePath(Box<[PathSegment]>);

impl FlattenNodePath {
    pub fn as_ref(&self) -> ResponsePathRef<'_> {
        ResponsePathRef(&self.0)
    }

    pub fn as_slice(&self) -> &[PathSegment] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<Vec<PathSegment>> for FlattenNodePath {
    fn from(segments: Vec<PathSegment>) -> Self {
        Self(segments.into_boxed_slice())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MergePaths(Box<[FlattenNodePath]>);

impl MergePaths {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = ResponsePathRef<'_>> + '_ {
        self.0.iter().map(FlattenNodePath::as_ref)
    }
}

impl From<Vec<FlattenNodePath>> for MergePaths {
    fn from(paths: Vec<FlattenNodePath>) -> Self {
        Self(paths.into_boxed_slice())
    }
}

impl<'de> Deserialize<'de> for TypeCondition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        Ok(Self::from_names(names.iter().map(String::as_str)))
    }
}

impl Display for PathSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathSegment::Field(name) => write!(f, "{name}"),
            PathSegment::TypeCondition(condition) => {
                write!(f, "|[")?;
                for (i, name) in condition.names.iter().enumerate() {
                    if i > 0 {
                        write!(f, "|")?;
                    }
                    write!(f, "{name}")?;
                }
                write!(f, "]")
            }
            PathSegment::List => write!(f, "@"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResponsePathRef<'a>(&'a [PathSegment]);

impl<'a> ResponsePathRef<'a> {
    pub fn as_slice(&self) -> &'a [PathSegment] {
        self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Display for ResponsePathRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut steps = self.0.iter().peekable();
        while let Some(step) = steps.next() {
            write!(f, "{step}")?;
            // A type condition belongs to the step before it, so no separator is added.
            if let Some(peeked) = steps.peek() {
                if !matches!(peeked, PathSegment::TypeCondition(_)) {
                    write!(f, ".")?;
                }
            }
        }
        Ok(())
    }
}

impl Display for FlattenNodePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.as_ref(), f)
    }
}

impl From<&MergePath> for FlattenNodePath {
    fn from(path: &MergePath) -> Self {
        path.inner
            .iter()
            .map(|segment| match segment {
                Segment::TypeCondition(type_names, _) => PathSegment::TypeCondition(Box::new(
                    TypeCondition::from_names(type_names.iter().map(String::as_str)),
                )),
                Segment::Field(field_seg, _args_hash, _) => {
                    PathSegment::Field(field_seg.response_key().into())
                }
                Segment::List => PathSegment::List,
            })
            .collect::<Vec<_>>()
            .into()
    }
}

impl From<MergePath> for FlattenNodePath {
    fn from(path: MergePath) -> Self {
        (&path).into()
    }
}

impl From<&MergePath> for Vec<FetchNodePathSegment> {
    fn from(value: &MergePath) -> Self {
        value
            .inner
            .iter()
            .filter_map(|path_segment| match path_segment {
                Segment::TypeCondition(type_names, _) => {
                    Some(FetchNodePathSegment::TypenameEquals(type_names.clone()))
                }
                Segment::Field(field_seg, _args_hash, _) => Some(FetchNodePathSegment::Key(
                    field_seg.response_key().to_string(),
                )),
                Segment::List => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod type_condition_tests {
    use super::*;

    #[test]
    fn names_are_sorted_and_deduplicated() {
        let messy = TypeCondition::from_names(["User", "Book", "User"]);
        let tidy = TypeCondition::from_names(["Book", "User"]);

        assert_eq!(
            messy.names(),
            tidy.names(),
            "not normalized on construction"
        );
        assert_eq!(messy, tidy, "equality depends on that normalization");

        let hash = |c: &TypeCondition| {
            use std::hash::{DefaultHasher, Hash, Hasher};
            let mut h = DefaultHasher::new();
            c.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash(&messy), hash(&tidy), "hashing depends on it too");

        assert_eq!(
            serde_json::to_string(&messy).expect("serializes"),
            r#"["Book","User"]"#,
            "the serialized order must not depend on how the condition was built"
        );
    }

    #[test]
    fn deserialization_normalizes_too() {
        let from_json: TypeCondition =
            serde_json::from_str(r#"["User","Book","User"]"#).expect("deserializes");
        assert_eq!(from_json, TypeCondition::from_names(["Book", "User"]));
        assert_eq!(
            from_json.names().len(),
            2,
            "duplicate survived deserialization"
        );
    }

    #[test]
    fn a_path_round_trips_through_an_unordered_condition() {
        let expected: FlattenNodePath = vec![
            PathSegment::Field("media".into()),
            PathSegment::TypeCondition(Box::new(TypeCondition::from_names(["Book", "User"]))),
        ]
        .into();

        let back: FlattenNodePath =
            serde_json::from_str(r#"[{"Field":"media"},{"TypeCondition":["User","Book","User"]}]"#)
                .expect("deserializes");
        assert_eq!(back, expected);
    }
}

#[cfg(test)]
#[cfg(target_pointer_width = "64")]
mod stored_size_tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn a_stored_step_stays_small() {
        assert_eq!(
            size_of::<PathSegment>(),
            24,
            "a stored path step is {} B",
            size_of::<PathSegment>()
        );
    }
}
