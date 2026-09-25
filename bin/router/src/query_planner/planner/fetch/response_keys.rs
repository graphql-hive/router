//! Response keys of the fields the plan adds for itself, like `@requires` fields and keys.
//!
//! A value is a field with its arguments, on some object. Two different values on one object
//! can't share a response key, even when two different fetches write them, because they land
//! in the same object of the response. So each value the plan adds gets its key here, once,
//! before any fetches are merged: the field's name when nothing else on that object uses it,
//! `_internal_qp_alias_N` otherwise. The client's fields keep their keys. Steps that read a
//! value in their input, or that sit under it in the response, read it from the same key.
//!
//! Objects are told apart by the fields and arguments on the way to them, not by type
//! conditions, lists or `@skip`/`@include`. That can put objects together that never meet,
//! like the ones under `... on Cat` and `... on Dog`, which only costs an alias.
//!
//! So fields of two steps can only clash when the client's own fields do: one key, two
//! values, on what's one object here. Like `thumbnail(width: 100)` and
//! `thumbnail(width: 200)` of photos under `... on Aquatics` and `... on Reptiles`. The
//! client may do that, the objects are apart in its query, but a fetch that gets all those
//! photos at once can't. It's rare, and the optimizer only has to check merges when it
//! happens.

use std::collections::{HashMap, HashSet};

use petgraph::graph::NodeIndex;

use crate::query_planner::{
    ast::{
        merge_path::{MergePath, Segment},
        selection_item::SelectionItem,
        selection_set::SelectionSet,
    },
    graph::{edge::Edge, Graph},
    planner::{
        fetch::{error::FetchGraphError, fetch_graph::FetchGraph},
        tree::query_tree_node::QueryTreeNode,
    },
};

const ALIAS_PREFIX: &str = "_internal_qp_alias_";

type ObjectId = u32;
const ROOT: ObjectId = 0;

/// A field name and the hash of its arguments.
type Value = (String, u64);

#[derive(Default)]
struct ResponseKeys {
    objects: HashMap<(ObjectId, Value), ObjectId>,
    /// `(object, value, key)` for every field the client asked for.
    client: HashSet<(ObjectId, Value, String)>,
    /// The values each key holds on each object.
    taken: HashMap<(ObjectId, String), HashSet<Value>>,
    /// The keys given to the plan's own values.
    internal: HashMap<(ObjectId, Value), String>,
    next_alias: usize,
    /// See the module docs.
    client_keys_clash: bool,
}

impl ResponseKeys {
    fn object(&mut self, parent: ObjectId, value: &Value) -> ObjectId {
        let next = self.objects.len() as ObjectId + 1;
        *self.objects.entry((parent, value.clone())).or_insert(next)
    }

    fn object_at(&mut self, path: &MergePath) -> ObjectId {
        path.inner
            .iter()
            .fold(ROOT, |object, segment| match segment {
                Segment::Field(field, args_hash, _) => {
                    self.object(object, &(field.field_name().to_string(), *args_hash))
                }
                _ => object,
            })
    }

    fn is_free(&self, object: ObjectId, key: &str, value: &Value) -> bool {
        self.taken
            .get(&(object, key.to_string()))
            .is_none_or(|values| values.iter().all(|taken| taken == value))
    }

    /// The key of a value the plan adds itself.
    fn internal_key(&mut self, object: ObjectId, value: &Value) -> String {
        if let Some(key) = self.internal.get(&(object, value.clone())) {
            return key.clone();
        }
        let key = if self.is_free(object, &value.0, value) {
            value.0.clone()
        } else {
            loop {
                let alias = format!("{ALIAS_PREFIX}{}", self.next_alias);
                self.next_alias += 1;
                if self.is_free(object, &alias, value) {
                    break alias;
                }
            }
        };
        self.taken
            .entry((object, key.clone()))
            .or_default()
            .insert(value.clone());
        self.internal.insert((object, value.clone()), key.clone());
        key
    }

    /// Takes the keys of the client's fields, the tree's children. Requirements are the plan's.
    fn take_client_keys(
        &mut self,
        graph: &Graph,
        node: &QueryTreeNode,
        object: ObjectId,
    ) -> Result<(), FetchGraphError> {
        for child in &node.children {
            let name = match child
                .edge_from_parent
                .map(|edge| graph.edge(edge))
                .transpose()?
            {
                Some(Edge::FieldMove(field)) => Some(field.name.as_str()),
                Some(Edge::ReentryMove(field)) => Some(field.name.as_str()),
                _ => None,
            };
            let child_object = match name {
                Some(name) => {
                    let value = (
                        name.to_string(),
                        child.selection_arguments().map_or(0, |a| a.hash_u64()),
                    );
                    let key = child.selection_alias().unwrap_or(name).to_string();
                    let values = self.taken.entry((object, key.clone())).or_default();
                    values.insert(value.clone());
                    self.client_keys_clash |= values.len() > 1;
                    self.client.insert((object, value.clone(), key));
                    self.object(object, &value)
                }
                None => object,
            };
            self.take_client_keys(graph, child, child_object)?;
        }
        Ok(())
    }

    /// Gives the plan's fields in a step's output their keys.
    fn key_output(&mut self, selection_set: &mut SelectionSet, object: ObjectId) {
        for item in selection_set.items.iter_mut() {
            match item {
                SelectionItem::Field(field) => {
                    let value = (field.name.clone(), field.arguments_hash());
                    let key = field.selection_identifier().to_string();
                    if !field.name.starts_with("__")
                        && !self.client.contains(&(object, value.clone(), key))
                    {
                        let key = self.internal_key(object, &value);
                        field.alias = (key != field.name).then_some(key);
                    }
                    let child = self.object(object, &value);
                    self.key_output(&mut field.selections, child);
                }
                SelectionItem::InlineFragment(fragment) => {
                    self.key_output(&mut fragment.selections, object)
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    /// Makes a step's input read the plan's fields from their keys, like
    /// `price: _internal_qp_alias_0` for a `price` that got an alias.
    fn key_input(&mut self, selection_set: &mut SelectionSet, object: ObjectId) {
        for item in selection_set.items.iter_mut() {
            match item {
                SelectionItem::Field(field) => {
                    let value = (field.name.clone(), field.arguments_hash());
                    if let Some(key) = self.internal.get(&(object, value.clone())) {
                        if *key != field.name {
                            field.alias = Some(field.name.clone());
                            field.name = key.clone();
                        }
                    }
                    let child = self.object(object, &value);
                    self.key_input(&mut field.selections, child);
                }
                SelectionItem::InlineFragment(fragment) => {
                    self.key_input(&mut fragment.selections, object)
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    /// The response path with the keys of the plan's fields on the way, when any of them got
    /// an alias.
    fn key_path(&mut self, path: &MergePath) -> Option<MergePath> {
        let mut object = ROOT;
        let mut segments = path.inner.to_vec();
        let mut changed = false;
        for segment in segments.iter_mut() {
            let Segment::Field(field, args_hash, _) = segment else {
                continue;
            };
            let value = (field.field_name().to_string(), *args_hash);
            if field.alias.is_none()
                && !self
                    .client
                    .contains(&(object, value.clone(), value.0.clone()))
            {
                if let Some(key) = self.internal.get(&(object, value.clone())) {
                    if *key != value.0 {
                        field.alias = Some(key.clone());
                        changed = true;
                    }
                }
            }
            object = self.object(object, &value);
        }
        changed.then(|| MergePath::new(segments))
    }
}

impl FetchGraph {
    /// See the module docs.
    pub(crate) fn give_internal_fields_their_keys(
        &mut self,
        graph: &Graph,
        tree_root: &QueryTreeNode,
    ) -> Result<(), FetchGraphError> {
        let mut keys = ResponseKeys::default();
        keys.take_client_keys(graph, tree_root, ROOT)?;
        self.client_keys_clash = keys.client_keys_clash;

        let steps: Vec<(NodeIndex, ObjectId)> = self
            .graph
            .node_indices()
            .map(|index| {
                (
                    index,
                    keys.object_at(self.graph[index].response_path.path()),
                )
            })
            .collect();

        for (index, object) in &steps {
            for (_, selection_set) in self.graph[*index].output.iter_selections_mut() {
                keys.key_output(selection_set, *object);
            }
        }
        for (index, object) in &steps {
            for (_, selection_set) in self.graph[*index].input.iter_selections_mut() {
                keys.key_input(selection_set, *object);
            }
        }
        for (index, _) in &steps {
            if let Some(path) = keys.key_path(self.graph[*index].response_path.path()) {
                self.graph[*index].response_path = self.locations.get(&path);
            }
        }

        Ok(())
    }
}
