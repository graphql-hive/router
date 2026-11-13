use std::collections::BTreeSet;
use std::fmt::Display;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{borrow::Cow, ops::Range};

use super::state::PathList;
use crate::executor::utils::traverse::PathTrie;

use crate::query_planner::ast::merge_path::{MergePath, Segment};

use super::FetchNodePathSegment;

/// Uses the tagged format that `lib/node-addon` and `hive-expose-query-plan` expect:
/// `{"Field": name}`, `{"TypeCondition": [..]}`, and a plain `"@"` for a list step.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PathSegment {
    /// Shared, so the tries built from a plan's paths point at the same names instead of copying them
    Field(Arc<str>),
    /// Behind a pointer, which keeps every path step small.
    TypeCondition(Arc<TypeCondition>),
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

/// One copy of every field name and type condition, for all the paths of a plan. A big plan has
/// thousands of paths over a few dozen names.
#[derive(Default)]
pub(crate) struct SharedSteps {
    names: ahash::HashSet<Arc<str>>,
    conditions: ahash::HashSet<Arc<TypeCondition>>,
}

impl SharedSteps {
    pub(crate) fn name(&mut self, name: &Arc<str>) -> Arc<str> {
        let mut name = Arc::clone(name);
        share(&mut self.names, &mut name);
        name
    }

    pub(crate) fn condition(&mut self, condition: &Arc<TypeCondition>) -> Arc<TypeCondition> {
        let mut condition = Arc::clone(condition);
        share(&mut self.conditions, &mut condition);
        condition
    }
}

fn share<T: Eq + std::hash::Hash + ?Sized>(shared: &mut ahash::HashSet<Arc<T>>, item: &mut Arc<T>) {
    match shared.get(item) {
        Some(existing) => *item = Arc::clone(existing),
        None => {
            shared.insert(Arc::clone(item));
        }
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

impl PathList for MergePaths {
    fn len(&self) -> usize {
        MergePaths::len(self)
    }

    fn to_paths(&self) -> Cow<'_, [FlattenNodePath]> {
        Cow::Borrowed(&self.0)
    }
}

/// A flatten node's path in an executable plan, as the trie walks use. A trie of one path keeps
/// just the path, with the plan's shared names.
#[derive(Debug, Clone)]
pub struct TriePath(PathTrie);

impl TriePath {
    pub(crate) fn new(path: &[PathSegment], shared: &mut SharedSteps) -> Self {
        Self(PathTrie::sharing([path], shared))
    }

    pub fn trie(&self) -> &PathTrie {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.path_len(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn to_path(&self) -> FlattenNodePath {
        self.0.path(0).into()
    }
}

impl Display for TriePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_path().fmt(f)
    }
}

impl Serialize for TriePath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_path().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TriePath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let path = FlattenNodePath::deserialize(deserializer)?;
        Ok(Self(PathTrie::new([path.as_slice()])))
    }
}

/// A batch alias's merge paths in an executable plan: its run of the paths of the trie the whole
/// batch shares, see [`super::EntityBatch::trie`]
#[derive(Debug, Clone)]
pub struct TriePaths {
    trie: Arc<PathTrie>,
    start: u32,
    len: u32,
}

impl TriePaths {
    pub(crate) fn new(trie: Arc<PathTrie>, paths: Range<usize>) -> Self {
        Self {
            trie,
            start: paths.start as u32,
            len: paths.len() as u32,
        }
    }

    pub fn trie(&self) -> &Arc<PathTrie> {
        &self.trie
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The alias's paths among the ones of [`Self::trie`]
    pub fn range(&self) -> Range<usize> {
        self.start as usize..(self.start + self.len) as usize
    }

    /// The alias's path `index`
    pub fn path(&self, index: usize) -> FlattenNodePath {
        self.trie.path(self.range().start + index).into()
    }

    /// How many steps the alias's path `index` has
    pub fn path_len(&self, index: usize) -> usize {
        self.trie.path_len(self.range().start + index)
    }
}

impl PathList for TriePaths {
    fn len(&self) -> usize {
        TriePaths::len(self)
    }

    fn to_paths(&self) -> Cow<'_, [FlattenNodePath]> {
        Cow::Owned((0..self.len()).map(|index| self.path(index)).collect())
    }
}

impl Serialize for TriePaths {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_paths().serialize(serializer)
    }
}

impl From<Vec<FlattenNodePath>> for TriePaths {
    /// A trie of their own, [`super::EntityBatch::build_tries`] puts an alias's together with
    /// the other ones of its batch
    fn from(paths: Vec<FlattenNodePath>) -> Self {
        let trie = PathTrie::new(paths.iter().map(FlattenNodePath::as_slice));
        Self::new(Arc::new(trie), 0..paths.len())
    }
}

/// Gives the alias a trie of its own. [`super::EntityBatch::build_tries`] puts them together.
impl<'de> Deserialize<'de> for TriePaths {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Vec::<FlattenNodePath>::deserialize(deserializer)?.into())
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
                Segment::TypeCondition(type_names, _) => {
                    PathSegment::TypeCondition(std::sync::Arc::new(TypeCondition::from_names(
                        type_names.iter().map(String::as_str),
                    )))
                }
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
                    Some(FetchNodePathSegment::TypenameEquals((**type_names).clone()))
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
            PathSegment::TypeCondition(std::sync::Arc::new(TypeCondition::from_names([
                "Book", "User",
            ]))),
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
