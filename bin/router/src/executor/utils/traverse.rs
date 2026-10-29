use std::sync::Arc;

use ahash::HashMap as AHashMap;

use crate::query_planner::planner::plan_nodes::{PathSegment, TypeCondition};

use crate::executor::{
    introspection::schema::{PossibleTypes, SchemaMetadata},
    response::{graphql_error::GraphQLErrorPath, value::Value},
    utils::consts::TYPENAME_FIELD_NAME,
};

fn entity_satisfies_any_type_condition(
    possible_types: &PossibleTypes,
    type_name: &str,
    condition: &TypeCondition,
) -> bool {
    condition
        .names()
        .iter()
        .any(|name| possible_types.entity_satisfies_type_condition(type_name, name))
}

pub fn traverse_and_callback_mut<'a, Callback>(
    current_data: &mut Value<'a>,
    remaining_path: &[PathSegment],
    schema_metadata: &SchemaMetadata,
    current_error_path: Option<GraphQLErrorPath>,
    callback: &mut Callback,
) where
    Callback: FnMut(&mut Value<'a>, Option<GraphQLErrorPath>),
{
    let mut error_path = current_error_path;
    walk_mut(
        current_data,
        remaining_path,
        schema_metadata,
        &mut error_path,
        callback,
    );
}

fn push_index(error_path: &mut Option<GraphQLErrorPath>, index: usize) {
    if let Some(error_path) = error_path.as_mut() {
        error_path.push_index(index);
    }
}

fn push_field(error_path: &mut Option<GraphQLErrorPath>, field: &str) {
    if let Some(error_path) = error_path.as_mut() {
        error_path.push_field(field);
    }
}

fn pop(error_path: &mut Option<GraphQLErrorPath>) {
    if let Some(error_path) = error_path.as_mut() {
        error_path.pop();
    }
}

/// Walks `remaining_path` while carrying a single error path. Steps are added as the walk goes
/// deeper and removed as it comes back, instead of copying the path built so far at every step.
/// Only the callback needs to own a path, so only the callback allocates one.
fn walk_mut<'a, Callback>(
    current_data: &mut Value<'a>,
    remaining_path: &[PathSegment],
    schema_metadata: &SchemaMetadata,
    error_path: &mut Option<GraphQLErrorPath>,
    callback: &mut Callback,
) where
    Callback: FnMut(&mut Value<'a>, Option<GraphQLErrorPath>),
{
    if remaining_path.is_empty() {
        if let Value::Array(arr) = current_data {
            // If the path is empty, we call the callback on each item in the array
            // We iterate because we want the entity objects directly
            for (index, item) in arr.iter_mut().enumerate() {
                push_index(error_path, index);
                callback(item, error_path.clone());
                pop(error_path);
            }
        } else {
            // If the path is empty and current_data is not an array, just call the callback
            callback(current_data, error_path.clone());
        }
        return;
    }

    let Some((first, rest_of_path)) = remaining_path.split_first() else {
        return;
    };

    match first {
        PathSegment::List => {
            // If the key is List, we expect current_data to be an array
            if let Value::Array(arr) = current_data {
                for (index, item) in arr.iter_mut().enumerate() {
                    push_index(error_path, index);
                    walk_mut(item, rest_of_path, schema_metadata, error_path, callback);
                    pop(error_path);
                }
            }
        }
        PathSegment::Field(field_name) => {
            // If the key is Field, we expect current_data to be an object
            if let Value::Object(map) = current_data {
                let field_name: &str = field_name.as_ref();
                if let Some(next_data) = Value::object_get_mut(map, field_name) {
                    push_field(error_path, field_name);
                    walk_mut(
                        next_data,
                        rest_of_path,
                        schema_metadata,
                        error_path,
                        callback,
                    );
                    pop(error_path);
                }
            }
        }
        PathSegment::TypeCondition(type_condition) => {
            // If the key is Cast, we expect current_data to be an object or an array
            if let Value::Object(obj) = current_data {
                let maybe_type_name =
                    Value::object_get(obj, TYPENAME_FIELD_NAME).and_then(Value::as_str);

                if maybe_type_name.is_none_or(|type_name| {
                    entity_satisfies_any_type_condition(
                        &schema_metadata.possible_types,
                        type_name,
                        type_condition,
                    )
                }) {
                    // A type condition does not add a path step.
                    walk_mut(
                        current_data,
                        rest_of_path,
                        schema_metadata,
                        error_path,
                        callback,
                    );
                }
            } else if let Value::Array(arr) = current_data {
                // If the current data is an array, we need to check each item
                for (index, item) in arr.iter_mut().enumerate() {
                    push_index(error_path, index);
                    // Pass `remaining_path` rather than `rest_of_path`, so the type condition
                    // is checked again for each item.
                    walk_mut(item, remaining_path, schema_metadata, error_path, callback);
                    pop(error_path);
                }
            }
        }
    }
}

/// From this many fields on, a trie node goes over the object's keys once instead of searching
/// the object for every field.
// ponytail: picked from the batch_paths bench, not tuned per object width
const FIELDS_BY_NAME_FROM: usize = 8;

/// The paths of a fetch merged into a tree, so one walk covers all of them and the steps they
/// share are walked once.
///
/// Each path sees its values in the same order a walk of that path alone gives them, so hashes
/// collected with [`PathTrie::walk`] line up with the values [`PathTrie::walk_mut`] gives later.
///
/// Plan nodes keep the tries of their paths, built when the plan is made executable, so it's
/// done once per plan. The names are shared with the paths, not copied.
#[derive(Debug, Clone)]
pub struct PathTrie {
    nodes: Vec<TrieNode>,
    /// The node each path ends at, in the order the paths were given
    ends: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
struct TrieNode {
    /// A path ends here
    end: bool,
    fields: Vec<(Arc<str>, usize)>,
    /// `fields` by name, when there are enough of them
    fields_by_name: Option<AHashMap<Arc<str>, usize>>,
    list: Option<usize>,
    type_conditions: Vec<(Arc<TypeCondition>, usize)>,
}

impl TrieNode {
    fn has_children(&self) -> bool {
        !self.fields.is_empty() || self.list.is_some() || !self.type_conditions.is_empty()
    }
}

fn satisfies_type_condition(
    object: &[(&str, Value<'_>)],
    condition: &TypeCondition,
    possible_types: &PossibleTypes,
) -> bool {
    Value::object_get(object, TYPENAME_FIELD_NAME)
        .and_then(Value::as_str)
        .is_none_or(|name| entity_satisfies_any_type_condition(possible_types, name, condition))
}

impl PathTrie {
    /// Identical paths end at the same node.
    pub fn new<'p>(paths: impl IntoIterator<Item = &'p [PathSegment]>) -> Self {
        let mut nodes = vec![TrieNode::default()];
        let mut ends = Vec::new();
        for path in paths {
            let mut node = 0;
            for segment in path {
                let existing = match segment {
                    PathSegment::Field(name) => nodes[node]
                        .fields
                        .iter()
                        .find(|(existing, _)| existing == name)
                        .map(|(_, child)| *child),
                    PathSegment::List => nodes[node].list,
                    PathSegment::TypeCondition(condition) => nodes[node]
                        .type_conditions
                        .iter()
                        .find(|(existing, _)| existing == condition)
                        .map(|(_, child)| *child),
                };
                node = match existing {
                    Some(child) => child,
                    None => {
                        let child = nodes.len();
                        nodes.push(TrieNode::default());
                        let parent = &mut nodes[node];
                        match segment {
                            PathSegment::Field(name) => {
                                parent.fields.push((Arc::clone(name), child))
                            }
                            PathSegment::List => parent.list = Some(child),
                            PathSegment::TypeCondition(condition) => {
                                parent.type_conditions.push((Arc::clone(condition), child))
                            }
                        }
                        child
                    }
                };
            }
            nodes[node].end = true;
            ends.push(node);
        }
        for node in &mut nodes {
            if node.fields.len() >= FIELDS_BY_NAME_FROM {
                node.fields_by_name = Some(node.fields.iter().cloned().collect());
            }
        }
        Self { nodes, ends }
    }

    /// The node each path ends at, in the order [`Self::new`] got them
    pub fn ends(&self) -> &[usize] {
        &self.ends
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Calls `callback` for every value at the end of every path, with the node the path ends at
    /// and the objects that hold the fields walked to get there (nearest last).
    pub fn walk<'d, 'v>(
        &self,
        data: &'d Value<'v>,
        possible_types: &PossibleTypes,
        callback: &mut impl FnMut(usize, &'d Value<'v>, &[&'d Value<'v>]),
    ) {
        self.walk_node(0, data, possible_types, &mut Vec::new(), callback);
    }

    /// Inlined, so most paths reach their end without a call. A call costs more than the
    /// callback when a node has nothing below it.
    #[inline(always)]
    fn walk_node<'d, 'v>(
        &self,
        index: usize,
        data: &'d Value<'v>,
        possible_types: &PossibleTypes,
        objects: &mut Vec<&'d Value<'v>>,
        callback: &mut impl FnMut(usize, &'d Value<'v>, &[&'d Value<'v>]),
    ) {
        let node = &self.nodes[index];
        if node.end {
            if let Value::Array(items) = data {
                for item in items {
                    callback(index, item, objects);
                }
            } else {
                callback(index, data, objects);
            }
        }
        if node.has_children() {
            self.walk_below(node, data, possible_types, objects, callback);
        }
    }

    fn walk_below<'d, 'v>(
        &self,
        node: &TrieNode,
        data: &'d Value<'v>,
        possible_types: &PossibleTypes,
        objects: &mut Vec<&'d Value<'v>>,
        callback: &mut impl FnMut(usize, &'d Value<'v>, &[&'d Value<'v>]),
    ) {
        match data {
            Value::Object(object) if !node.fields.is_empty() => {
                objects.push(data);
                if let Some(fields_by_name) = &node.fields_by_name {
                    // a key the object has twice is walked twice, a search would find it once
                    for (key, child) in object {
                        if let Some(&child_index) = fields_by_name.get(*key) {
                            self.walk_node(child_index, child, possible_types, objects, callback);
                        }
                    }
                } else {
                    for (name, child_index) in &node.fields {
                        if let Some(child) = Value::object_get(object, name) {
                            self.walk_node(*child_index, child, possible_types, objects, callback);
                        }
                    }
                }
                objects.pop();
            }
            Value::Array(items) => {
                if let Some(list) = node.list {
                    for item in items {
                        self.walk_node(list, item, possible_types, objects, callback);
                    }
                }
            }
            _ => {}
        }
        for (condition, child_index) in &node.type_conditions {
            self.walk_type_condition(
                condition,
                *child_index,
                data,
                possible_types,
                objects,
                callback,
            );
        }
    }

    /// A type condition doesn't add a step. On a list it's checked for each item.
    fn walk_type_condition<'d, 'v>(
        &self,
        condition: &TypeCondition,
        index: usize,
        data: &'d Value<'v>,
        possible_types: &PossibleTypes,
        objects: &mut Vec<&'d Value<'v>>,
        callback: &mut impl FnMut(usize, &'d Value<'v>, &[&'d Value<'v>]),
    ) {
        match data {
            Value::Object(object) => {
                if satisfies_type_condition(object, condition, possible_types) {
                    self.walk_node(index, data, possible_types, objects, callback);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.walk_type_condition(
                        condition,
                        index,
                        item,
                        possible_types,
                        objects,
                        callback,
                    );
                }
            }
            _ => {}
        }
    }

    /// Like [`Self::walk`], but with mutable values and, when `error_path` is given, the path to
    /// each value. A value is given out before the walk goes below it, so a path shouldn't go
    /// through what's written at the end of another one.
    pub fn walk_mut<'a>(
        &self,
        data: &mut Value<'a>,
        possible_types: &PossibleTypes,
        error_path: Option<GraphQLErrorPath>,
        callback: &mut impl FnMut(usize, &mut Value<'a>, Option<&GraphQLErrorPath>),
    ) {
        let mut error_path = error_path;
        self.walk_node_mut(0, data, possible_types, &mut error_path, callback);
    }

    /// See [`Self::walk_node`]
    #[inline(always)]
    fn walk_node_mut<'a>(
        &self,
        index: usize,
        data: &mut Value<'a>,
        possible_types: &PossibleTypes,
        error_path: &mut Option<GraphQLErrorPath>,
        callback: &mut impl FnMut(usize, &mut Value<'a>, Option<&GraphQLErrorPath>),
    ) {
        let node = &self.nodes[index];
        if node.end {
            if let Value::Array(items) = &mut *data {
                for (item_index, item) in items.iter_mut().enumerate() {
                    push_index(error_path, item_index);
                    callback(index, item, error_path.as_ref());
                    pop(error_path);
                }
            } else {
                callback(index, data, error_path.as_ref());
            }
        }
        if node.has_children() {
            self.walk_below_mut(node, data, possible_types, error_path, callback);
        }
    }

    fn walk_below_mut<'a>(
        &self,
        node: &TrieNode,
        data: &mut Value<'a>,
        possible_types: &PossibleTypes,
        error_path: &mut Option<GraphQLErrorPath>,
        callback: &mut impl FnMut(usize, &mut Value<'a>, Option<&GraphQLErrorPath>),
    ) {
        match &mut *data {
            Value::Object(object) if !node.fields.is_empty() => {
                if let Some(fields_by_name) = &node.fields_by_name {
                    for (key, child) in object.iter_mut() {
                        if let Some(&child_index) = fields_by_name.get(*key) {
                            push_field(error_path, key);
                            self.walk_node_mut(
                                child_index,
                                child,
                                possible_types,
                                error_path,
                                callback,
                            );
                            pop(error_path);
                        }
                    }
                } else {
                    for (name, child_index) in &node.fields {
                        if let Some(child) = Value::object_get_mut(object, name) {
                            push_field(error_path, name);
                            self.walk_node_mut(
                                *child_index,
                                child,
                                possible_types,
                                error_path,
                                callback,
                            );
                            pop(error_path);
                        }
                    }
                }
            }
            Value::Array(items) => {
                if let Some(list) = node.list {
                    for (item_index, item) in items.iter_mut().enumerate() {
                        push_index(error_path, item_index);
                        self.walk_node_mut(list, item, possible_types, error_path, callback);
                        pop(error_path);
                    }
                }
            }
            _ => {}
        }
        for (condition, child_index) in &node.type_conditions {
            self.walk_type_condition_mut(
                condition,
                *child_index,
                data,
                possible_types,
                error_path,
                callback,
            );
        }
    }

    fn walk_type_condition_mut<'a>(
        &self,
        condition: &TypeCondition,
        index: usize,
        data: &mut Value<'a>,
        possible_types: &PossibleTypes,
        error_path: &mut Option<GraphQLErrorPath>,
        callback: &mut impl FnMut(usize, &mut Value<'a>, Option<&GraphQLErrorPath>),
    ) {
        match data {
            Value::Object(object) => {
                if satisfies_type_condition(object, condition, possible_types) {
                    self.walk_node_mut(index, data, possible_types, error_path, callback);
                }
            }
            Value::Array(items) => {
                for (item_index, item) in items.iter_mut().enumerate() {
                    push_index(error_path, item_index);
                    self.walk_type_condition_mut(
                        condition,
                        index,
                        item,
                        possible_types,
                        error_path,
                        callback,
                    );
                    pop(error_path);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::query_planner::planner::plan_nodes::{FlattenNodePath, PathSegment, TypeCondition};

    /// `@` means a list step, `|A|B` means a type condition, and anything else is a field.
    fn path(steps: &[&str]) -> FlattenNodePath {
        steps
            .iter()
            .map(|step| match *step {
                "@" => PathSegment::List,
                s if s.starts_with('|') => PathSegment::TypeCondition(std::sync::Arc::new(
                    TypeCondition::from_names(s.trim_start_matches('|').split('|')),
                )),
                field => PathSegment::Field(field.into()),
            })
            .collect::<Vec<_>>()
            .into()
    }

    use crate::executor::{
        introspection::schema::SchemaMetadata,
        response::{
            graphql_error::{GraphQLErrorPath, GraphQLErrorPathSegment},
            value::Value,
        },
    };

    #[test]
    /**
     * Collect error paths for each item in a list at one level
     * E.g. for data { items: [ {...}, {...} ] } and path ["items", List]
     * we should collect paths ["items", 0] and ["items", 1]
     */
    fn test_collect_error_paths_one_level() {
        let mut data = Value::Object(vec![(
            "items",
            Value::Array(vec![
                Value::Object(vec![("id", Value::String("1".into()))]),
                Value::Object(vec![("id", Value::String("2".into()))]),
            ]),
        )]);
        let path = path(&["items", "@"]);
        let mut collected = vec![];
        super::traverse_and_callback_mut(
            &mut data,
            path.as_slice(),
            &SchemaMetadata::default(),
            Some(GraphQLErrorPath::default()),
            &mut |_item, error_path| {
                collected.push(error_path.unwrap());
            },
        );
        assert_eq!(collected.len(), 2);
        assert_eq!(
            collected[0].segments,
            vec![
                GraphQLErrorPathSegment::String("items".into()),
                GraphQLErrorPathSegment::Index(0)
            ]
        );
        assert_eq!(
            collected[1].segments,
            vec![
                GraphQLErrorPathSegment::String("items".into()),
                GraphQLErrorPathSegment::Index(1)
            ]
        );
    }

    #[test]
    /**
     * Collect error paths for each item in a list at two levels
     * E.g. for data { users: [ { posts: [ {...}, {...} ] }, { posts: [ {...} ] } ] } and path ["users", List, "posts", List]
     * we should collect paths ["users", 0, "posts", 0], ["users", 0, "posts", 1], and ["users", 1, "posts", 0]
     */
    fn test_collect_error_paths_two_levels() {
        let mut data = Value::Object(vec![(
            "users",
            Value::Array(vec![
                Value::Object(vec![
                    ("id", Value::String("1".into())),
                    (
                        "posts",
                        Value::Array(vec![
                            Value::Object(vec![("id", Value::String("a".into()))]),
                            Value::Object(vec![("id", Value::String("b".into()))]),
                        ]),
                    ),
                ]),
                Value::Object(vec![
                    ("id", Value::String("2".into())),
                    (
                        "posts",
                        Value::Array(vec![Value::Object(vec![("id", Value::String("c".into()))])]),
                    ),
                ]),
            ]),
        )]);
        let path = path(&["users", "@", "posts", "@"]);
        let mut collected = vec![];
        super::traverse_and_callback_mut(
            &mut data,
            path.as_slice(),
            &SchemaMetadata::default(),
            Some(GraphQLErrorPath::default()),
            &mut |_item, error_path| {
                collected.push(error_path.unwrap());
            },
        );
        assert_eq!(collected.len(), 3);
        assert_eq!(
            collected[0].segments,
            vec![
                GraphQLErrorPathSegment::String("users".into()),
                GraphQLErrorPathSegment::Index(0),
                GraphQLErrorPathSegment::String("posts".into()),
                GraphQLErrorPathSegment::Index(0),
            ]
        );
        assert_eq!(
            collected[1].segments,
            vec![
                GraphQLErrorPathSegment::String("users".into()),
                GraphQLErrorPathSegment::Index(0),
                GraphQLErrorPathSegment::String("posts".into()),
                GraphQLErrorPathSegment::Index(1),
            ]
        );
        assert_eq!(
            collected[2].segments,
            vec![
                GraphQLErrorPathSegment::String("users".into()),
                GraphQLErrorPathSegment::Index(1),
                GraphQLErrorPathSegment::String("posts".into()),
                GraphQLErrorPathSegment::Index(0),
            ]
        );
    }

    /// The walker shares one error path across siblings instead of giving each level its own
    /// copy. If a step is added and not removed again, it would leak from one entity into the
    /// next. Type conditions are the tricky case, because they go deeper without adding a step.
    #[test]
    fn error_paths_survive_type_conditions_and_sibling_branches() {
        let mut data = Value::Object(vec![(
            "media",
            Value::Array(vec![
                Value::Object(vec![
                    ("__typename", Value::String("Book".into())),
                    (
                        "pages",
                        Value::Array(vec![Value::Object(vec![("id", Value::String("a".into()))])]),
                    ),
                ]),
                Value::Object(vec![
                    ("__typename", Value::String("Book".into())),
                    (
                        "pages",
                        Value::Array(vec![Value::Object(vec![("id", Value::String("b".into()))])]),
                    ),
                ]),
            ]),
        )]);

        let path = path(&["media", "@", "|Book", "pages", "@"]);
        let mut collected = vec![];
        super::traverse_and_callback_mut(
            &mut data,
            path.as_slice(),
            &SchemaMetadata::default(),
            Some(GraphQLErrorPath::default()),
            &mut |_item, error_path| {
                collected.push(error_path.expect("error path is tracked").segments);
            },
        );

        use crate::executor::response::graphql_error::GraphQLErrorPathSegment::{Index, String};
        assert_eq!(
            collected,
            vec![
                vec![
                    String("media".into()),
                    Index(0),
                    String("pages".into()),
                    Index(0)
                ],
                vec![
                    String("media".into()),
                    Index(1),
                    String("pages".into()),
                    Index(0)
                ],
            ],
            "the second entity's path must not inherit steps from the first"
        );
    }

    #[test]
    fn trie_matches_multi_type_cast() {
        let data = Value::Object(vec![("__typename", Value::String("Book".into()))]);
        let path = path(&["|Book|User"]);
        let trie = super::PathTrie::new([path.as_slice()]);
        let mut matched = false;

        trie.walk(&data, &Default::default(), &mut |_, _, _| {
            matched = true;
        });

        assert!(matched);
    }

    #[test]
    fn trie_rejects_non_matching_multi_type_cast() {
        let data = Value::Object(vec![("__typename", Value::String("Magazine".into()))]);
        let path = path(&["|Book|User"]);
        let trie = super::PathTrie::new([path.as_slice()]);
        let mut matched = false;

        trie.walk(&data, &Default::default(), &mut |_, _, _| {
            matched = true;
        });

        assert!(!matched);
    }

    /// Hashes collected with one walk are matched to values by position in the other, so every
    /// path has to see the same values in the same order as a walk of that path alone.
    #[test]
    fn trie_walks_each_path_like_a_walk_of_it_alone() {
        let data: Value = sonic_rs::from_str(
            r#"{
                "items": [
                    {"__typename":"Book","a":{"id":1},"b":{"id":2},
                     "f0":0,"f1":1,"f2":2,"f3":3,"f4":4,"f5":5,"f6":6,"f7":7,"f8":8,"f9":9},
                    null,
                    {"__typename":"User","a":{"id":3},"b":{"id":4},"f3":{"x":1}},
                    [{"__typename":"Book","b":{"id":5}}],
                    {"__typename":"Magazine","a":[{"id":6},null]}
                ]
            }"#,
        )
        .unwrap();
        let mut paths = vec![
            path(&["items", "@", "a"]),
            path(&["items", "@"]),
            path(&["items", "@", "|Book", "b"]),
            path(&["items", "|Book|User"]),
            path(&["items", "@", "a"]),
            path(&[]),
            path(&["items", "@", "missing"]),
        ];
        // enough fields on one node to go over the object's keys instead
        let fields: Vec<String> = (0..10).map(|i| format!("f{i}")).collect();
        paths.extend(fields.iter().map(|field| path(&["items", "@", field])));
        let metadata = SchemaMetadata::default();

        type Found = Vec<(String, Vec<GraphQLErrorPathSegment>)>;
        let alone: Vec<Found> = paths
            .iter()
            .map(|path| {
                let mut data = data.clone();
                let mut found = Vec::new();
                super::traverse_and_callback_mut(
                    &mut data,
                    path.as_slice(),
                    &metadata,
                    Some(GraphQLErrorPath::with_capacity(0)),
                    &mut |value, error_path| {
                        found.push((
                            sonic_rs::to_string(value).unwrap(),
                            error_path.unwrap().segments,
                        ))
                    },
                );
                found
            })
            .collect();

        let trie = super::PathTrie::new(paths.iter().map(|path| path.as_slice()));
        let ends = trie.ends();
        assert_eq!(ends[0], ends[4], "identical paths end at the same node");
        let mut by_node: Vec<Found> = vec![Vec::new(); trie.len()];
        let mut target = data.clone();
        trie.walk_mut(
            &mut target,
            &metadata.possible_types,
            Some(GraphQLErrorPath::with_capacity(0)),
            &mut |node, value, error_path| {
                by_node[node].push((
                    sonic_rs::to_string(value).unwrap(),
                    error_path.unwrap().segments.clone(),
                ))
            },
        );
        // the objects above each value are the ones holding the fields walked to get there
        let mut by_node_shared: Vec<Vec<(String, usize)>> = vec![Vec::new(); trie.len()];
        trie.walk(
            &data,
            &metadata.possible_types,
            &mut |node, value, objects| {
                by_node_shared[node].push((sonic_rs::to_string(value).unwrap(), objects.len()))
            },
        );

        for (index, (end, alone)) in ends.iter().zip(&alone).enumerate() {
            assert_eq!(&by_node[*end], alone, "walk_mut of path {}", paths[index]);
            let expected: Vec<(String, usize)> = alone
                .iter()
                .map(|(value, error_path)| {
                    let fields = error_path
                        .iter()
                        .filter(|segment| matches!(segment, GraphQLErrorPathSegment::String(_)))
                        .count();
                    (value.clone(), fields)
                })
                .collect();
            assert_eq!(
                by_node_shared[*end], expected,
                "walk of path {}",
                paths[index]
            );
        }
        assert!(
            alone.iter().map(Vec::len).sum::<usize>() > 20,
            "the paths find values"
        );
    }
}
