use std::sync::Arc;

use ahash::HashMap as AHashMap;

use crate::query_planner::planner::plan_nodes::{PathSegment, SharedSteps, TypeCondition};

use crate::executor::{
    introspection::schema::PossibleTypes,
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

/// From this many fields on, a trie node goes over the object's keys once instead of searching
/// the object for every field.
// ponytail: picked by benching, not tuned per object width
const FIELDS_BY_NAME_FROM: usize = 8;

/// No list step below a node, or nothing rare about it
const NONE: u32 = u32::MAX;

/// The paths of a fetch merged into a tree, so one walk covers all of them and the steps they
/// share are walked once.
///
/// Each path sees its values in the same order a walk of that path alone gives them, so hashes
/// collected with [`PathTrie::walk`] line up with the values [`PathTrie::walk_mut`] gives later.
///
/// Tries stay around as long as their plan is cached, and a big one has tens of thousands of
/// nodes. So a node is a few ranges into lists the whole trie shares, and a field step points at
/// its name by number.
#[derive(Debug, Clone)]
pub struct PathTrie {
    nodes: Box<[TrieNode]>,
    /// The field steps below every node, each node's next to each other
    fields: Box<[FieldStep]>,
    /// The field names of the trie. A plan's paths share their names, so most come once.
    names: Box<[Arc<str>]>,
    /// First the node each path ends at, in the order [`PathTrie::new`] got them. Then the
    /// paths that end at every node, each node's next to each other, in that order too. One
    /// list, since small tries are the most common and each list costs an allocation.
    ends_and_paths: Box<[u32]>,
    /// What only a few nodes have, and many tries have none of
    rare: Option<Box<RareParts>>,
}

#[derive(Debug, Clone)]
struct RareParts {
    nodes: Box<[Rare]>,
    /// The type conditions below every node that has some, each node's next to each other
    conditions: Box<[(Arc<TypeCondition>, u32)]>,
    /// The fields of nodes that have lots of them, by name
    fields_by_name: Box<[AHashMap<Arc<str>, u32>]>,
}

/// Lists are (start, length) in the trie's shared ones
#[derive(Debug, Clone, Copy)]
struct TrieNode {
    fields: (u32, u32),
    paths: (u32, u32),
    list: u32,
    /// Index into [`RareParts::nodes`], or `NONE`
    rare: u32,
}

#[derive(Debug, Clone, Copy)]
struct FieldStep {
    /// Index into [`PathTrie::names`]
    name: u32,
    child: u32,
}

#[derive(Debug, Clone, Copy)]
struct Rare {
    conditions: (u32, u32),
    /// Index into [`RareParts::fields_by_name`], or `NONE`
    fields_by_name: u32,
}

impl TrieNode {
    fn has_children(&self) -> bool {
        self.fields.1 != 0 || self.list != NONE || self.rare != NONE
    }

    fn has_fields(&self) -> bool {
        self.fields.1 != 0
    }

    fn has_paths(&self) -> bool {
        self.paths.1 != 0
    }
}

fn range<T>(items: &[T], (start, len): (u32, u32)) -> &[T] {
    &items[start as usize..][..len as usize]
}

/// A node while the trie is built. Its steps are linked lists in lists the whole build shares,
/// so a build allocates per trie, not per node.
#[derive(Clone, Copy)]
struct BuildNode {
    /// The first and the last of the node's field steps
    fields: (u32, u32),
    field_count: u32,
    list: u32,
    conditions: (u32, u32),
    condition_count: u32,
}

impl Default for BuildNode {
    fn default() -> Self {
        Self {
            fields: (NONE, NONE),
            field_count: 0,
            list: NONE,
            conditions: (NONE, NONE),
            condition_count: 0,
        }
    }
}

struct Linked<T> {
    item: T,
    next: u32,
}

/// Adds `item` after the last one of the list that `first_last` points at
fn push_linked<T>(items: &mut Vec<Linked<T>>, first_last: &mut (u32, u32), item: T) {
    let index = items.len() as u32;
    items.push(Linked { item, next: NONE });
    match first_last.1 {
        NONE => first_last.0 = index,
        last => items[last as usize].next = index,
    }
    first_last.1 = index;
}

fn linked<T>(items: &[Linked<T>], first: u32) -> impl Iterator<Item = &T> {
    std::iter::successors(items.get(first as usize), |item| {
        items.get(item.next as usize)
    })
    .map(|item| &item.item)
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
        Self::build(paths, None)
    }

    /// Like [`Self::new`], but the trie keeps the names and type conditions of `shared`, for
    /// paths that don't share theirs and are dropped once the trie is built.
    pub(crate) fn sharing<'p>(
        paths: impl IntoIterator<Item = &'p [PathSegment]>,
        shared: &mut SharedSteps,
    ) -> Self {
        Self::build(paths, Some(shared))
    }

    fn build<'p>(
        paths: impl IntoIterator<Item = &'p [PathSegment]>,
        mut shared: Option<&mut SharedSteps>,
    ) -> Self {
        let mut nodes = vec![BuildNode::default()];
        let mut steps: Vec<Linked<FieldStep>> = Vec::new();
        let mut type_conditions: Vec<Linked<(Arc<TypeCondition>, u32)>> = Vec::new();
        let mut names: Vec<Arc<str>> = Vec::new();
        let mut ends = Vec::new();
        for path in paths {
            let mut node = 0;
            for segment in path {
                let existing = match segment {
                    PathSegment::Field(name) => linked(&steps, nodes[node].fields.0)
                        .find(|step| names[step.name as usize] == *name)
                        .map(|step| step.child),
                    PathSegment::List => Some(nodes[node].list).filter(|&list| list != NONE),
                    PathSegment::TypeCondition(condition) => {
                        linked(&type_conditions, nodes[node].conditions.0)
                            .find(|(existing, _)| existing == condition)
                            .map(|(_, child)| *child)
                    }
                };
                if let Some(child) = existing {
                    node = child as usize;
                    continue;
                }
                let child = nodes.len() as u32;
                nodes.push(BuildNode::default());
                let parent = &mut nodes[node];
                match segment {
                    PathSegment::Field(name) => {
                        let name = &match shared.as_deref_mut() {
                            Some(shared) => shared.name(name),
                            None => Arc::clone(name),
                        };
                        // names a plan shares are found by pointer, any other one is only kept
                        // twice
                        let id = match names.iter().position(|known| Arc::ptr_eq(known, name)) {
                            Some(id) => id,
                            None => {
                                names.push(Arc::clone(name));
                                names.len() - 1
                            }
                        };
                        let step = FieldStep {
                            name: id as u32,
                            child,
                        };
                        push_linked(&mut steps, &mut parent.fields, step);
                        parent.field_count += 1;
                    }
                    PathSegment::TypeCondition(condition) => {
                        let condition = match shared.as_deref_mut() {
                            Some(shared) => shared.condition(condition),
                            None => Arc::clone(condition),
                        };
                        push_linked(
                            &mut type_conditions,
                            &mut parent.conditions,
                            (condition, child),
                        );
                        parent.condition_count += 1;
                    }
                    PathSegment::List => parent.list = child,
                }
                node = child as usize;
            }
            ends.push(node as u32);
        }

        // the paths of every node go after the ends, in the order we got them
        let path_count = ends.len();
        let mut path_starts = vec![0u32; nodes.len()];
        for &end in &ends {
            path_starts[end as usize] += 1;
        }
        let mut start = path_count as u32;
        for count in &mut path_starts {
            (*count, start) = (start, start + *count);
        }
        let mut path_ends = path_starts.clone();
        let mut ends_and_paths = vec![0u32; 2 * path_count];
        ends_and_paths[..path_count].copy_from_slice(&ends);
        for (path, &end) in ends.iter().enumerate() {
            let slot = &mut path_ends[end as usize];
            ends_and_paths[*slot as usize] = path as u32;
            *slot += 1;
        }

        let mut fields = Vec::with_capacity(steps.len());
        let mut rare = Vec::new();
        let mut conditions = Vec::new();
        let mut fields_by_name = Vec::new();
        let nodes = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let field_start = fields.len();
                fields.extend(linked(&steps, node.fields.0).copied());
                let by_name = if node.field_count as usize >= FIELDS_BY_NAME_FROM {
                    fields_by_name.push(
                        fields[field_start..]
                            .iter()
                            .map(|step: &FieldStep| {
                                (Arc::clone(&names[step.name as usize]), step.child)
                            })
                            .collect(),
                    );
                    fields_by_name.len() as u32 - 1
                } else {
                    NONE
                };
                let rare_index = if by_name != NONE || node.condition_count > 0 {
                    let condition_start = conditions.len() as u32;
                    conditions.extend(linked(&type_conditions, node.conditions.0).cloned());
                    rare.push(Rare {
                        conditions: (condition_start, node.condition_count),
                        fields_by_name: by_name,
                    });
                    rare.len() as u32 - 1
                } else {
                    NONE
                };
                TrieNode {
                    fields: (field_start as u32, node.field_count),
                    paths: (path_starts[index], path_ends[index] - path_starts[index]),
                    list: node.list,
                    rare: rare_index,
                }
            })
            .collect();
        Self {
            nodes,
            fields: fields.into(),
            names: names.into(),
            ends_and_paths: ends_and_paths.into(),
            rare: (!rare.is_empty()).then(|| {
                Box::new(RareParts {
                    nodes: rare.into(),
                    conditions: conditions.into(),
                    fields_by_name: fields_by_name.into(),
                })
            }),
        }
    }

    /// The node each path ends at, in the order [`Self::new`] got them
    pub fn ends(&self) -> &[u32] {
        &self.ends_and_paths[..self.ends_and_paths.len() / 2]
    }

    /// The paths that end at `node`, in the order [`Self::new`] got them
    pub fn paths_ending_at(&self, node: usize) -> &[u32] {
        range(&self.ends_and_paths, self.nodes[node].paths)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn rare(&self, node: &TrieNode) -> (&RareParts, &Rare) {
        let parts = self
            .rare
            .as_deref()
            .expect("a node with rare parts has them");
        (parts, &parts.nodes[node.rare as usize])
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
        if node.has_paths() {
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
        if node.rare != NONE {
            self.walk_below_rare(node, data, possible_types, objects, callback);
        } else {
            self.walk_steps(node, None, data, possible_types, objects, callback);
        }
    }

    /// Kept apart, so the nodes with only fields and lists don't pay for what few nodes have
    fn walk_below_rare<'d, 'v>(
        &self,
        node: &TrieNode,
        data: &'d Value<'v>,
        possible_types: &PossibleTypes,
        objects: &mut Vec<&'d Value<'v>>,
        callback: &mut impl FnMut(usize, &'d Value<'v>, &[&'d Value<'v>]),
    ) {
        let (parts, rare) = self.rare(node);
        let fields_by_name = parts.fields_by_name.get(rare.fields_by_name as usize);
        self.walk_steps(
            node,
            fields_by_name,
            data,
            possible_types,
            objects,
            callback,
        );
        for (condition, child_index) in range(&parts.conditions, rare.conditions) {
            self.walk_type_condition(
                condition,
                *child_index as usize,
                data,
                possible_types,
                objects,
                callback,
            );
        }
    }

    #[inline(always)]
    fn walk_steps<'d, 'v>(
        &self,
        node: &TrieNode,
        fields_by_name: Option<&AHashMap<Arc<str>, u32>>,
        data: &'d Value<'v>,
        possible_types: &PossibleTypes,
        objects: &mut Vec<&'d Value<'v>>,
        callback: &mut impl FnMut(usize, &'d Value<'v>, &[&'d Value<'v>]),
    ) {
        match data {
            Value::Object(object) if node.has_fields() => {
                objects.push(data);
                if let Some(fields_by_name) = fields_by_name {
                    // a key the object has twice is walked twice, a search would find it once
                    for (key, child) in object {
                        if let Some(&child_index) = fields_by_name.get(*key) {
                            let child_index = child_index as usize;
                            self.walk_node(child_index, child, possible_types, objects, callback);
                        }
                    }
                } else {
                    for step in range(&self.fields, node.fields) {
                        if let Some(child) =
                            Value::object_get(object, &self.names[step.name as usize])
                        {
                            let child_index = step.child as usize;
                            self.walk_node(child_index, child, possible_types, objects, callback);
                        }
                    }
                }
                objects.pop();
            }
            Value::Array(items) if node.list != NONE => {
                for item in items {
                    self.walk_node(node.list as usize, item, possible_types, objects, callback);
                }
            }
            _ => {}
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
        if node.has_paths() {
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
        if node.rare != NONE {
            self.walk_below_rare_mut(node, data, possible_types, error_path, callback);
        } else {
            self.walk_steps_mut(node, None, data, possible_types, error_path, callback);
        }
    }

    /// See [`Self::walk_below_rare`]
    fn walk_below_rare_mut<'a>(
        &self,
        node: &TrieNode,
        data: &mut Value<'a>,
        possible_types: &PossibleTypes,
        error_path: &mut Option<GraphQLErrorPath>,
        callback: &mut impl FnMut(usize, &mut Value<'a>, Option<&GraphQLErrorPath>),
    ) {
        let (parts, rare) = self.rare(node);
        let fields_by_name = parts.fields_by_name.get(rare.fields_by_name as usize);
        self.walk_steps_mut(
            node,
            fields_by_name,
            data,
            possible_types,
            error_path,
            callback,
        );
        for (condition, child_index) in range(&parts.conditions, rare.conditions) {
            self.walk_type_condition_mut(
                condition,
                *child_index as usize,
                data,
                possible_types,
                error_path,
                callback,
            );
        }
    }

    #[inline(always)]
    fn walk_steps_mut<'a>(
        &self,
        node: &TrieNode,
        fields_by_name: Option<&AHashMap<Arc<str>, u32>>,
        data: &mut Value<'a>,
        possible_types: &PossibleTypes,
        error_path: &mut Option<GraphQLErrorPath>,
        callback: &mut impl FnMut(usize, &mut Value<'a>, Option<&GraphQLErrorPath>),
    ) {
        match data {
            Value::Object(object) if node.has_fields() => {
                if let Some(fields_by_name) = fields_by_name {
                    for (key, child) in object.iter_mut() {
                        if let Some(&child_index) = fields_by_name.get(*key) {
                            push_field(error_path, key);
                            self.walk_node_mut(
                                child_index as usize,
                                child,
                                possible_types,
                                error_path,
                                callback,
                            );
                            pop(error_path);
                        }
                    }
                } else {
                    for step in range(&self.fields, node.fields) {
                        let name = &self.names[step.name as usize];
                        if let Some(child) = Value::object_get_mut(object, name) {
                            push_field(error_path, name);
                            self.walk_node_mut(
                                step.child as usize,
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
            Value::Array(items) if node.list != NONE => {
                let list = node.list as usize;
                for (item_index, item) in items.iter_mut().enumerate() {
                    push_index(error_path, item_index);
                    self.walk_node_mut(list, item, possible_types, error_path, callback);
                    pop(error_path);
                }
            }
            _ => {}
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

    /// Walks `path` with a trie of it alone, like a flatten fetch does.
    fn walk_alone<'a>(
        data: &mut Value<'a>,
        path: &FlattenNodePath,
        callback: &mut impl FnMut(&mut Value<'a>, Option<&GraphQLErrorPath>),
    ) {
        super::PathTrie::new([path.as_slice()]).walk_mut(
            data,
            &Default::default(),
            Some(GraphQLErrorPath::default()),
            &mut |_, value, error_path| callback(value, error_path),
        );
    }

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
        walk_alone(&mut data, &path, &mut |_item, error_path| {
            collected.push(error_path.unwrap().clone());
        });
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
        walk_alone(&mut data, &path, &mut |_item, error_path| {
            collected.push(error_path.unwrap().clone());
        });
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
        walk_alone(&mut data, &path, &mut |_item, error_path| {
            collected.push(error_path.expect("error path is tracked").segments.clone());
        });

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

    /// Hashes collected with one walk are matched to values by position in the other, so a trie
    /// of many paths has to walk each of them like a trie of that path alone.
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
                walk_alone(&mut data, path, &mut |value, error_path| {
                    found.push((
                        sonic_rs::to_string(value).unwrap(),
                        error_path.unwrap().segments.clone(),
                    ))
                });
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
            assert_eq!(
                &by_node[*end as usize], alone,
                "walk_mut of path {}",
                paths[index]
            );
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
                by_node_shared[*end as usize], expected,
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
