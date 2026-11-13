//! The selection sets of one planning run's fetch steps, in a few shared arrays.
//!
//! A selection set is a `ListId`, stable while the list grows. Every list belongs to one owner (a
//! fetch step's type, or the field or fragment it's under), so changing a list in place is safe,
//! the way it is with the planner's tree. Copying one is cheap: no strings, no maps, just numbers.
//!
//! Names are interned and so are arguments: two fields with equal arguments have the same
//! `ArgsId`, so comparing fields is comparing numbers.

use std::collections::BTreeSet;

use ahash::HashMap;

use super::{selection_tree::Sel, Interner, Kind, Span, Sym, NONE};
use crate::query_planner::ast::selection_set::{FieldSelection, InlineFragmentSelection};
use crate::query_planner::ast::{
    arguments::ArgumentsMap,
    merge_path::{Condition, MergePath, Segment},
    selection_item::SelectionItem,
    selection_set::{ResponseKeyConflict, SelectionSet},
};

pub type NodeId = u32;
pub type ArgsId = u32;

/// A selection set in the arena. No `==` on purpose: two lists are equal by what's in them,
/// see `Arena::lists_equal`, never by id.
#[derive(Clone, Copy, Debug)]
pub struct ListId(u32);

impl ListId {
    /// The id as a number, for keying things built from a list. Not for comparing lists.
    pub fn index(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub kind: Kind,
    /// The field's name, the fragment's type condition, or the spread's fragment name.
    pub name: Sym,
    pub alias: Sym,
    pub args: ArgsId,
    pub skip_if: Sym,
    pub include_if: Sym,
    /// See `FieldSelection::omit_from_response`.
    pub omit_from_response: bool,
    pub children: ListId,
}

#[derive(Default, Clone)]
pub struct Arena {
    strings: Interner,
    args: Vec<ArgumentsMap>,
    /// `ArgumentsMap::hash_u64` of each entry of `args`.
    args_hash: Vec<u64>,
    args_by_hash: HashMap<u64, Vec<ArgsId>>,
    nodes: Vec<Node>,
    lists: Vec<Span>,
    items: Vec<NodeId>,
}

impl std::fmt::Debug for Arena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Arena")
            .field("nodes", &self.nodes.len())
            .field("lists", &self.lists.len())
            .finish()
    }
}

impl Arena {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- strings and arguments

    #[inline]
    pub fn intern(&mut self, s: &str) -> Sym {
        self.strings.intern(s)
    }

    #[inline]
    pub fn str(&self, sym: Sym) -> &str {
        self.strings.str(sym)
    }

    /// The string, or `None` for `NONE`.
    pub fn opt_str(&self, sym: Sym) -> Option<&str> {
        (sym != NONE).then(|| self.str(sym))
    }

    fn intern_opt(&mut self, s: Option<&str>) -> Sym {
        s.map_or(NONE, |s| self.intern(s))
    }

    pub fn intern_args(&mut self, args: &ArgumentsMap) -> ArgsId {
        let hash = args.hash_u64();
        if let Some(ids) = self.args_by_hash.get(&hash) {
            if let Some(&id) = ids.iter().find(|&&id| self.args[id as usize] == *args) {
                return id;
            }
        }
        let id = self.args.len() as ArgsId;
        self.args.push(args.clone());
        self.args_hash.push(hash);
        self.args_by_hash.entry(hash).or_default().push(id);
        id
    }

    pub fn args(&self, id: ArgsId) -> Option<&ArgumentsMap> {
        (id != NONE).then(|| &self.args[id as usize])
    }

    /// `FieldSelection::arguments_hash`: 0 without arguments.
    pub fn args_hash(&self, id: ArgsId) -> u64 {
        if id == NONE {
            0
        } else {
            self.args_hash[id as usize]
        }
    }

    /// Like `FieldSelection::arguments()`, empty arguments are no arguments.
    fn same_args(&self, a: ArgsId, b: ArgsId) -> bool {
        let empty = |id: ArgsId| id == NONE || self.args[id as usize].is_empty();
        a == b || (empty(a) && empty(b))
    }

    // ---- nodes and lists

    #[inline]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    #[inline]
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id as usize]
    }

    pub fn new_list(&mut self) -> ListId {
        self.lists.push(Span::EMPTY);
        ListId((self.lists.len() - 1) as u32)
    }

    #[inline]
    pub fn items(&self, list: ListId) -> &[NodeId] {
        &self.items[self.lists[list.0 as usize].range()]
    }

    #[inline]
    pub fn items_mut(&mut self, list: ListId) -> &mut [NodeId] {
        let range = self.lists[list.0 as usize].range();
        &mut self.items[range]
    }

    #[inline]
    pub fn is_empty(&self, list: ListId) -> bool {
        self.lists[list.0 as usize].is_empty()
    }

    /// Makes `list` the last run of `items`, so it can grow in place. The old run is left
    /// behind, the arena only lives as long as one plan is built.
    fn make_last(&mut self, list: ListId) {
        let span = self.lists[list.0 as usize];
        if span.start + span.len == self.items.len() as u32 {
            return;
        }
        let start = self.items.len() as u32;
        self.items.extend_from_within(span.range());
        self.lists[list.0 as usize].start = start;
    }

    pub fn push(&mut self, list: ListId, node: NodeId) {
        self.make_last(list);
        self.items.push(node);
        self.lists[list.0 as usize].len += 1;
    }

    /// Replaces the items of `list`.
    pub fn set_items(&mut self, list: ListId, nodes: &[NodeId]) {
        let span = self.lists[list.0 as usize];
        if nodes.len() as u32 <= span.len {
            self.items[span.start as usize..span.start as usize + nodes.len()]
                .copy_from_slice(nodes);
        } else {
            let start = self.items.len() as u32;
            self.items.extend_from_slice(nodes);
            self.lists[list.0 as usize].start = start;
        }
        self.lists[list.0 as usize].len = nodes.len() as u32;
    }

    pub fn retain(&mut self, list: ListId, mut keep: impl FnMut(&Self, NodeId) -> bool) {
        let span = self.lists[list.0 as usize];
        let mut kept = 0;
        for index in span.range() {
            let id = self.items[index];
            if keep(self, id) {
                self.items[span.start as usize + kept] = id;
                kept += 1;
            }
        }
        self.lists[list.0 as usize].len = kept as u32;
    }

    pub fn add_node(&mut self, node: Node) -> NodeId {
        self.nodes.push(node);
        (self.nodes.len() - 1) as NodeId
    }

    pub fn new_field(
        &mut self,
        name: &str,
        alias: Option<&str>,
        args: Option<&ArgumentsMap>,
        condition: Option<&Condition>,
    ) -> NodeId {
        let node = Node {
            kind: Kind::Field,
            name: self.intern(name),
            alias: self.intern_opt(alias),
            args: args.map_or(NONE, |args| self.intern_args(args)),
            skip_if: self.intern_opt(condition.and_then(skip_of)),
            include_if: self.intern_opt(condition.and_then(include_of)),
            omit_from_response: false,
            children: self.new_list(),
        };
        self.add_node(node)
    }

    pub fn new_fragment(&mut self, type_condition: &str, condition: Option<&Condition>) -> NodeId {
        let node = Node {
            kind: Kind::InlineFragment,
            name: self.intern(type_condition),
            alias: NONE,
            args: NONE,
            skip_if: self.intern_opt(condition.and_then(skip_of)),
            include_if: self.intern_opt(condition.and_then(include_of)),
            omit_from_response: false,
            children: self.new_list(),
        };
        self.add_node(node)
    }

    /// A copy of `list` and everything below it, owned by no one yet.
    pub fn copy(&mut self, list: ListId) -> ListId {
        let copy = self.new_list();
        for index in 0..self.items(list).len() {
            let node = self.copy_node(self.items(list)[index]);
            self.push(copy, node);
        }
        copy
    }

    pub fn copy_node(&mut self, id: NodeId) -> NodeId {
        let mut node = *self.node(id);
        node.children = self.copy(node.children);
        self.add_node(node)
    }

    /// A copy of `list` from `other`, strings and arguments interned in this arena.
    pub fn copy_from(&mut self, other: &Arena, list: ListId) -> ListId {
        // collect first: pushing one by one would move the list each time a child list lands
        // after it
        let nodes: Vec<NodeId> = other
            .items(list)
            .iter()
            .map(|&id| self.copy_node_from(other, id))
            .collect();
        let copy = self.new_list();
        self.set_items(copy, &nodes);
        copy
    }

    fn copy_node_from(&mut self, other: &Arena, id: NodeId) -> NodeId {
        let children = self.copy_from(other, other.node(id).children);
        self.copy_shell_from(other, id, children)
    }

    /// The node `id` of `other`, without what's under it: `children` goes there instead.
    pub fn copy_shell_from(&mut self, other: &Arena, id: NodeId, children: ListId) -> NodeId {
        let source = other.node(id);
        let node = Node {
            kind: source.kind,
            name: self.intern(other.str(source.name)),
            alias: self.intern_opt(other.opt_str(source.alias)),
            args: other
                .args(source.args)
                .map_or(NONE, |args| self.intern_args(args)),
            skip_if: self.intern_opt(other.opt_str(source.skip_if)),
            include_if: self.intern_opt(other.opt_str(source.include_if)),
            omit_from_response: source.omit_from_response,
            children,
        };
        self.add_node(node)
    }

    /// Selections of a `SelectionTree`, a field set say, copied in.
    pub fn copy_from_tree<'a>(&mut self, items: impl Iterator<Item = Sel<'a>>) -> ListId {
        let nodes: Vec<NodeId> = items
            .map(|sel| {
                let children = self.copy_from_tree(sel.children());
                let attributes = sel.attributes();
                let node = Node {
                    kind: sel.kind(),
                    name: self.intern(sel.name()),
                    alias: self.intern_opt(attributes.and_then(|a| a.alias.as_deref())),
                    args: attributes
                        .and_then(|a| a.arguments.as_ref())
                        .map_or(NONE, |args| self.intern_args(args)),
                    skip_if: self.intern_opt(sel.condition().and_then(skip_of)),
                    include_if: self.intern_opt(sel.condition().and_then(include_of)),
                    omit_from_response: false,
                    children,
                };
                self.add_node(node)
            })
            .collect();
        self.list_of(&nodes)
    }

    /// A new list of `nodes`.
    pub fn list_of(&mut self, nodes: &[NodeId]) -> ListId {
        let list = self.new_list();
        self.set_items(list, nodes);
        list
    }

    /// `{ __typename }`
    pub fn typename_list(&mut self) -> ListId {
        let typename = self.new_field("__typename", None, None, None);
        self.list_of(&[typename])
    }

    pub fn new_spread(&mut self, fragment_name: &str) -> NodeId {
        let node = Node {
            kind: Kind::FragmentSpread,
            name: self.intern(fragment_name),
            alias: NONE,
            args: NONE,
            skip_if: NONE,
            include_if: NONE,
            omit_from_response: false,
            children: self.new_list(),
        };
        self.add_node(node)
    }

    // ---- the tree

    // ponytail: only `PlanDocument::from_tree` (benches, tests) still brings a tree in
    pub fn import(&mut self, selection_set: &SelectionSet) -> ListId {
        let list = self.new_list();
        for item in &selection_set.items {
            let node = self.import_item(item);
            self.push(list, node);
        }
        list
    }

    fn import_item(&mut self, item: &SelectionItem) -> NodeId {
        let node = match item {
            SelectionItem::Field(field) => Node {
                kind: Kind::Field,
                name: self.intern(&field.name),
                alias: self.intern_opt(field.alias.as_deref()),
                args: field
                    .arguments
                    .as_ref()
                    .map_or(NONE, |args| self.intern_args(args)),
                skip_if: self.intern_opt(field.skip_if.as_deref()),
                include_if: self.intern_opt(field.include_if.as_deref()),
                omit_from_response: field.omit_from_response,
                children: self.import(&field.selections),
            },
            SelectionItem::InlineFragment(fragment) => Node {
                kind: Kind::InlineFragment,
                name: self.intern(&fragment.type_condition),
                alias: NONE,
                args: NONE,
                skip_if: self.intern_opt(fragment.skip_if.as_deref()),
                include_if: self.intern_opt(fragment.include_if.as_deref()),
                omit_from_response: false,
                children: self.import(&fragment.selections),
            },
            SelectionItem::FragmentSpread(name) => Node {
                kind: Kind::FragmentSpread,
                name: self.intern(name),
                alias: NONE,
                args: NONE,
                skip_if: NONE,
                include_if: NONE,
                omit_from_response: false,
                children: self.new_list(),
            },
        };
        self.add_node(node)
    }

    // ponytail: plugin hooks still take a tree, built from here on demand. Goes when they don't
    pub fn export(&self, list: ListId) -> SelectionSet {
        SelectionSet {
            items: self
                .items(list)
                .iter()
                .map(|&id| self.export_node(id))
                .collect(),
        }
    }

    pub fn export_node(&self, id: NodeId) -> SelectionItem {
        let node = self.node(id);
        let owned = |sym: Sym| self.opt_str(sym).map(str::to_string);
        match node.kind {
            Kind::Field => SelectionItem::Field(FieldSelection {
                name: self.str(node.name).to_string(),
                selections: self.export(node.children),
                alias: owned(node.alias),
                arguments: self.args(node.args).cloned(),
                skip_if: owned(node.skip_if),
                include_if: owned(node.include_if),
                omit_from_response: node.omit_from_response,
            }),
            Kind::InlineFragment => SelectionItem::InlineFragment(InlineFragmentSelection {
                type_condition: self.str(node.name).to_string(),
                selections: self.export(node.children),
                skip_if: owned(node.skip_if),
                include_if: owned(node.include_if),
            }),
            Kind::FragmentSpread => SelectionItem::FragmentSpread(self.str(node.name).to_string()),
        }
    }

    // ---- reading

    /// The alias, or the name when there's no alias.
    #[inline]
    pub fn response_key(&self, id: NodeId) -> &str {
        let node = self.node(id);
        self.str(if node.alias != NONE {
            node.alias
        } else {
            node.name
        })
    }

    pub fn condition(&self, id: NodeId) -> Option<Condition> {
        let node = self.node(id);
        match (self.opt_str(node.skip_if), self.opt_str(node.include_if)) {
            (Some(skip), Some(include)) => Some(Condition::SkipAndInclude {
                skip: skip.to_string(),
                include: include.to_string(),
            }),
            (Some(skip), None) => Some(Condition::Skip(skip.to_string())),
            (None, Some(include)) => Some(Condition::Include(include.to_string())),
            (None, None) => None,
        }
    }

    /// `field_condition_equal`: the node has exactly this condition.
    pub fn condition_is(&self, id: NodeId, condition: Option<&Condition>) -> bool {
        let node = self.node(id);
        let skip = self.opt_str(node.skip_if);
        let include = self.opt_str(node.include_if);
        match condition {
            None => skip.is_none() && include.is_none(),
            Some(Condition::Include(variable)) => {
                include == Some(variable.as_str()) && skip.is_none()
            }
            Some(Condition::Skip(variable)) => skip == Some(variable.as_str()) && include.is_none(),
            Some(Condition::SkipAndInclude {
                skip: s,
                include: i,
            }) => skip == Some(s.as_str()) && include == Some(i.as_str()),
        }
    }

    pub fn set_condition(&mut self, id: NodeId, condition: Option<&Condition>) {
        let skip_if = self.intern_opt(condition.and_then(skip_of));
        let include_if = self.intern_opt(condition.and_then(include_of));
        let node = self.node_mut(id);
        node.skip_if = skip_if;
        node.include_if = include_if;
    }

    fn same_condition(&self, a: NodeId, b: NodeId) -> bool {
        let (a, b) = (self.node(a), self.node(b));
        a.skip_if == b.skip_if && a.include_if == b.include_if
    }

    /// `FieldSelection`'s `==`: name, alias and arguments.
    fn same_field(&self, a: NodeId, b: NodeId) -> bool {
        let (na, nb) = (self.node(a), self.node(b));
        na.name == nb.name && na.alias == nb.alias && self.same_args(na.args, nb.args)
    }

    pub fn variable_usages(&self, list: ListId, usages: &mut BTreeSet<String>) {
        for &id in self.items(list) {
            let node = self.node(id);
            if let Some(args) = self.args(node.args) {
                for value in args.values() {
                    usages.extend(value.variable_usages());
                }
            }
            for sym in [node.include_if, node.skip_if] {
                if let Some(variable) = self.opt_str(sym) {
                    usages.insert(variable.to_string());
                }
            }
            self.variable_usages(node.children, usages);
        }
    }

    // ---- what the planner does with selection sets

    /// `read_as_written`: `price: _internal_qp_alias_0` becomes `_internal_qp_alias_0: price`.
    pub fn read_as_written(&mut self, list: ListId) {
        for index in 0..self.items(list).len() {
            let id = self.items(list)[index];
            let node = self.node_mut(id);
            if node.kind == Kind::Field && node.alias != NONE {
                std::mem::swap(&mut node.alias, &mut node.name);
            }
            let children = node.children;
            self.read_as_written(children);
        }
    }

    /// `SelectionSet`'s `==`: the same items in the same order. Fields compare by name, alias,
    /// arguments, conditions and what's under them.
    pub fn lists_equal(&self, a: ListId, b: ListId) -> bool {
        let (a, b) = (self.items(a), self.items(b));
        a.len() == b.len() && a.iter().zip(b).all(|(&a, &b)| self.items_equal(a, b))
    }

    fn items_equal(&self, a: NodeId, b: NodeId) -> bool {
        let (na, nb) = (self.node(a), self.node(b));
        match (na.kind, nb.kind) {
            (Kind::Field, Kind::Field) => {
                self.same_field(a, b)
                    && self.same_condition(a, b)
                    && self.lists_equal(na.children, nb.children)
            }
            (Kind::InlineFragment, Kind::InlineFragment) => {
                na.name == nb.name
                    && self.same_condition(a, b)
                    && self.lists_equal(na.children, nb.children)
            }
            (Kind::FragmentSpread, Kind::FragmentSpread) => na.name == nb.name,
            _ => false,
        }
    }

    /// `SemanticEq`: the same items in any order. Fields compare by name, alias, arguments,
    /// conditions, `omit_from_response` and what's under them.
    pub fn same_selections(&self, a: ListId, b: ListId) -> bool {
        let (a, b) = (self.items(a), self.items(b));
        if a.len() != b.len() {
            return false;
        }
        let mut matched = vec![false; b.len()];
        a.iter().all(|&left| {
            let found = b
                .iter()
                .enumerate()
                .position(|(index, &right)| !matched[index] && self.same_item(left, right));
            found.map(|index| matched[index] = true).is_some()
        })
    }

    fn same_item(&self, a: NodeId, b: NodeId) -> bool {
        let (na, nb) = (self.node(a), self.node(b));
        match (na.kind, nb.kind) {
            (Kind::Field, Kind::Field) => {
                self.same_field(a, b)
                    && self.same_condition(a, b)
                    && na.omit_from_response == nb.omit_from_response
                    && self.same_selections(na.children, nb.children)
            }
            (Kind::InlineFragment, Kind::InlineFragment) => {
                na.name == nb.name
                    && self.same_condition(a, b)
                    && self.same_selections(na.children, nb.children)
            }
            (Kind::FragmentSpread, Kind::FragmentSpread) => na.name == nb.name,
            _ => false,
        }
    }

    /// `selection_items_are_subset_of`: every item of `target` is in `source`.
    pub fn is_subset(&self, source: ListId, target: ListId) -> bool {
        self.items(target).iter().all(|&t| {
            self.items(source)
                .iter()
                .any(|&s| self.item_is_subset(s, t))
        })
    }

    fn item_is_subset(&self, source: NodeId, target: NodeId) -> bool {
        let (s, t) = (self.node(source), self.node(target));
        match (s.kind, t.kind) {
            (Kind::Field, Kind::Field) => {
                self.same_field(source, target)
                    && self.same_condition(source, target)
                    && self.is_empty(s.children) == self.is_empty(t.children)
                    && self.is_subset(s.children, t.children)
            }
            (Kind::InlineFragment, Kind::InlineFragment) => {
                s.name == t.name
                    && self.same_condition(source, target)
                    && self.is_subset(s.children, t.children)
            }
            _ => false,
        }
    }

    /// `is_subset` with `target` still a tree: every item of `target` is in `source`.
    #[cfg(test)]
    pub fn contains_tree(&self, source: ListId, target: &SelectionSet) -> bool {
        target.items.iter().all(|t| {
            self.items(source)
                .iter()
                .any(|&s| self.item_contains_tree(s, t))
        })
    }

    #[cfg(test)]
    fn item_contains_tree(&self, source: NodeId, target: &SelectionItem) -> bool {
        let s = self.node(source);
        let same_condition = |skip_if: &Option<String>, include_if: &Option<String>| {
            self.opt_str(s.skip_if) == skip_if.as_deref()
                && self.opt_str(s.include_if) == include_if.as_deref()
        };
        match (s.kind, target) {
            (Kind::Field, SelectionItem::Field(t)) => {
                self.str(s.name) == t.name
                    && self.opt_str(s.alias) == t.alias.as_deref()
                    && self.args(s.args).filter(|args| !args.is_empty()) == t.arguments()
                    && same_condition(&t.skip_if, &t.include_if)
                    && self.is_empty(s.children) == t.selections.is_empty()
                    && self.contains_tree(s.children, &t.selections)
            }
            (Kind::InlineFragment, SelectionItem::InlineFragment(t)) => {
                self.str(s.name) == t.type_condition
                    && same_condition(&t.skip_if, &t.include_if)
                    && self.contains_tree(s.children, &t.selections)
            }
            _ => false,
        }
    }

    /// `merge_selection_set`: adds what `source` has and `target` doesn't, copied.
    pub fn merge(
        &mut self,
        target: ListId,
        source: ListId,
        as_first: bool,
    ) -> Result<(), ResponseKeyConflict> {
        if self.is_empty(source) {
            return Ok(());
        }

        let mut pending = Vec::new();
        for source_index in 0..self.items(source).len() {
            let source_item = self.items(source)[source_index];
            let mut found = None;
            for &target_item in self.items(target) {
                let (s, t) = (self.node(source_item), self.node(target_item));
                match (s.kind, t.kind) {
                    (Kind::Field, Kind::Field)
                        if self.response_key(source_item) == self.response_key(target_item)
                            && (s.name != t.name || !self.same_args(s.args, t.args)) =>
                    {
                        return Err(ResponseKeyConflict(
                            self.response_key(source_item).to_string(),
                        ));
                    }
                    (Kind::Field, Kind::Field)
                        if self.same_field(source_item, target_item)
                            && self.same_condition(source_item, target_item) =>
                    {
                        found = Some(target_item);
                        break;
                    }
                    (Kind::InlineFragment, Kind::InlineFragment)
                        if s.name == t.name && self.same_condition(source_item, target_item) =>
                    {
                        found = Some(target_item);
                        break;
                    }
                    _ => {}
                }
            }

            match found {
                Some(target_item) => {
                    let s = *self.node(source_item);
                    // A skipped `__typename` is only there for an empty object. If the client
                    // asked for `__typename`, keep the visible one.
                    if s.kind == Kind::Field && self.str(s.name) == "__typename" {
                        self.node_mut(target_item).omit_from_response &= s.omit_from_response;
                    }
                    let target_children = self.node(target_item).children;
                    self.merge(target_children, s.children, as_first)?;
                }
                None => pending.push(self.copy_node(source_item)),
            }
        }

        if !pending.is_empty() {
            if as_first {
                pending.extend_from_slice(self.items(target));
                self.set_items(target, &pending);
            } else {
                for node in pending {
                    self.push(target, node);
                }
            }
        }

        Ok(())
    }

    fn segment_selects(&self, segment: &Segment, id: NodeId) -> bool {
        let node = self.node(id);
        match (segment, node.kind) {
            (Segment::TypeCondition(type_names, condition), Kind::InlineFragment) => {
                type_names.contains(self.str(node.name))
                    && self.condition_is(id, condition.as_ref())
            }
            (Segment::Field(field, args_hash, condition), Kind::Field) => {
                self.response_key(id) == field.response_key()
                    && self.args_hash(node.args) == *args_hash
                    && self.condition_is(id, condition.as_ref())
            }
            _ => false,
        }
    }

    /// `find_selection_set_by_path`
    pub fn find_by_path(&self, list: ListId, path: &MergePath) -> Option<ListId> {
        let mut current = list;
        for segment in path.inner.iter() {
            if matches!(segment, Segment::List) {
                continue;
            }
            let id = *self
                .items(current)
                .iter()
                .find(|&&id| self.segment_selects(segment, id))?;
            if self.node(id).kind == Kind::FragmentSpread {
                return None;
            }
            current = self.node(id).children;
        }
        Some(current)
    }

    /// `response_keys_conflict`
    pub fn response_keys_conflict(&self, a: ListId, b: ListId) -> bool {
        self.any_field_through_fragments(a, &|a_field| {
            self.any_field_through_fragments(b, &|b_field| {
                let (na, nb) = (self.node(a_field), self.node(b_field));
                self.response_key(a_field) == self.response_key(b_field)
                    && (na.name != nb.name
                        || !self.same_args(na.args, nb.args)
                        || self.response_keys_conflict(na.children, nb.children))
            })
        })
    }

    fn any_field_through_fragments(
        &self,
        list: ListId,
        predicate: &impl Fn(NodeId) -> bool,
    ) -> bool {
        self.items(list).iter().any(|&id| match self.node(id).kind {
            Kind::Field => predicate(id),
            Kind::InlineFragment => {
                self.any_field_through_fragments(self.node(id).children, predicate)
            }
            Kind::FragmentSpread => false,
        })
    }
}

fn skip_of(condition: &Condition) -> Option<&str> {
    match condition {
        Condition::Skip(variable) => Some(variable),
        Condition::SkipAndInclude { skip, .. } => Some(skip),
        Condition::Include(_) => None,
    }
}

fn include_of(condition: &Condition) -> Option<&str> {
    match condition {
        Condition::Include(variable) => Some(variable),
        Condition::SkipAndInclude { include, .. } => Some(include),
        Condition::Skip(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::query::{Definition, OperationDefinition};

    use super::*;
    use crate::query_planner::ast::selection_set::{
        find_selection_set_by_path, merge_selection_set, response_keys_conflict,
        selection_items_are_subset_of,
    };
    use crate::query_planner::utils::parsing::parse_operation;

    fn set(query: &str) -> SelectionSet {
        if query.is_empty() {
            return SelectionSet::default();
        }
        match parse_operation(query).definitions.into_iter().next() {
            Some(Definition::Operation(OperationDefinition::SelectionSet(s))) => s.into(),
            _ => panic!("expected a selection set"),
        }
    }

    const SETS: &[&str] = &[
        "{ a }",
        "{ a b }",
        "{ a { b } }",
        "{ a { c } d }",
        "{ x: a }",
        "{ a(x: 1) }",
        "{ a(x: 2) }",
        "{ a(x: 1) { b } }",
        "{ a @skip(if: $s) }",
        "{ a @include(if: $i) { b } }",
        "{ ... on T { a } }",
        "{ ... on T @include(if: $i) { a } }",
        "{ ... on U { a { b } } }",
        "{ __typename a }",
        "{ a { b { c } } ... on T { a(x: 1) } }",
        "",
    ];

    #[test]
    fn round_trips() {
        let mut arena = Arena::new();
        for query in SETS {
            let tree = set(query);
            let list = arena.import(&tree);
            assert_eq!(arena.export(list).to_string(), tree.to_string(), "{query}");
            let copy = arena.copy(list);
            assert_eq!(arena.export(copy).to_string(), tree.to_string(), "{query}");
            let mut other = Arena::new();
            other.intern("shifts the symbols");
            let copy = other.copy_from(&arena, list);
            assert_eq!(other.export(copy).to_string(), tree.to_string(), "{query}");
        }
    }

    #[test]
    fn same_answers_as_the_tree() {
        for a in SETS {
            for b in SETS {
                for as_first in [false, true] {
                    let (ta, tb) = (set(a), set(b));
                    let mut arena = Arena::new();
                    let (la, lb) = (arena.import(&ta), arena.import(&tb));

                    assert_eq!(
                        arena.is_subset(la, lb),
                        selection_items_are_subset_of(&ta.items, &tb.items),
                        "subset {a} {b}"
                    );
                    assert_eq!(arena.lists_equal(la, lb), ta == tb, "equal {a} {b}");
                    assert_eq!(
                        arena.contains_tree(la, &tb),
                        selection_items_are_subset_of(&ta.items, &tb.items),
                        "contains_tree {a} {b}"
                    );
                    assert_eq!(
                        arena.response_keys_conflict(la, lb),
                        response_keys_conflict(&ta, &tb),
                        "conflict {a} {b}"
                    );

                    let mut merged = ta.clone();
                    let tree_result = merge_selection_set(&mut merged, &tb, as_first);
                    let arena_result = arena.merge(la, lb, as_first);
                    assert_eq!(tree_result.is_ok(), arena_result.is_ok(), "merge {a} {b}");
                    if tree_result.is_ok() {
                        assert_eq!(
                            arena.export(la).to_string(),
                            merged.to_string(),
                            "merge {a} {b} {as_first}"
                        );
                        // the source is left alone
                        assert_eq!(arena.export(lb).to_string(), tb.to_string());
                    }
                }
            }
        }
    }

    #[test]
    fn finds_by_path() {
        use crate::query_planner::ast::merge_path::FieldPathSegment;
        use std::rc::Rc;

        let tree = set("{ a { b @skip(if: $s) { c } ... on T { d { e } } } x: a(y: 1) { f } }");
        let mut arena = Arena::new();
        let list = arena.import(&tree);
        let args = ArgumentsMap::from((
            "y".to_string(),
            crate::query_planner::ast::value::Value::Int(1),
        ));
        let field = |name: &str, alias: Option<&str>, hash: u64, cond: Option<Condition>| {
            Segment::Field(
                Rc::new(FieldPathSegment::new(name.into(), alias.map(Into::into))),
                hash,
                cond,
            )
        };
        let paths = [
            vec![field("a", None, 0, None)],
            vec![
                field("a", None, 0, None),
                Segment::List,
                field("b", None, 0, Some(Condition::Skip("s".into()))),
            ],
            vec![field("a", None, 0, None), field("b", None, 0, None)],
            vec![
                field("a", None, 0, None),
                Segment::TypeCondition(Rc::new(["T".to_string()].into()), None),
                field("d", None, 0, None),
            ],
            vec![field("a", Some("x"), args.hash_u64(), None)],
            vec![field("a", Some("x"), 0, None)],
        ];
        for path in paths {
            let path = MergePath::new(path);
            let tree_found = find_selection_set_by_path(&tree, &path).map(|s| s.to_string());
            let arena_found = arena
                .find_by_path(list, &path)
                .map(|l| arena.export(l).to_string());
            assert_eq!(arena_found, tree_found, "{path}");
        }
    }
}
