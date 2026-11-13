//! A normalized selection set in flat arrays, read-only, the way the planner's walker reads it.
//!
//! A selection is an index. A field's alias and arguments sit behind one `Arc`, so a path that
//! goes through the field shares them instead of copying a map of arguments on every step.

use std::sync::Arc;

use super::arena::{Arena, ListId};
use super::plan_document::PlanDocument;

use super::{Interner, Kind, SelId, Span, Sym};
use crate::query_planner::ast::{
    arguments::ArgumentsMap,
    merge_path::Condition,
    operation::{OperationDefinition, VariableDefinition},
    selection_item::SelectionItem,
    selection_set::SelectionSet,
};
use crate::query_planner::state::supergraph_state::OperationKind;

/// The alias and arguments of a field from the operation, or from a field set.
#[derive(Debug, Clone, Default)]
pub struct SelectionAttributes {
    pub alias: Option<String>,
    pub arguments: Option<ArgumentsMap>,
}

impl PartialEq for SelectionAttributes {
    fn eq(&self, other: &Self) -> bool {
        self.alias == other.alias && self.arguments == other.arguments
    }
}

/// Same alias and arguments. The same field is the same `Arc`, so that's checked first.
pub fn same_attributes(
    a: Option<&Arc<SelectionAttributes>>,
    b: Option<&Arc<SelectionAttributes>>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
        _ => false,
    }
}

/// The operation the planner plans: normalized, its selections flat.
#[derive(Debug, Clone)]
pub struct PlanOperation {
    pub kind: OperationKind,
    pub selections: SelectionTree,
    // ponytail: still the tree's variables, until variables are flat too
    pub variables: Option<Vec<VariableDefinition>>,
}

impl Default for PlanOperation {
    fn default() -> Self {
        PlanOperation {
            kind: OperationKind::Query,
            selections: SelectionTree::default(),
            variables: None,
        }
    }
}

impl PlanOperation {
    /// From the normalized operation: the same selections, read the walker's way.
    pub fn from_document(document: &PlanDocument) -> Self {
        PlanOperation {
            kind: document.kind.clone().unwrap_or(OperationKind::Query),
            selections: SelectionTree::from_arena(&document.arena, document.selections),
            variables: document.variables.clone(),
        }
    }

    /// From a tree operation: a plugin's, the authorization's, or a test's.
    pub fn from_tree(operation: &OperationDefinition) -> Self {
        PlanOperation {
            kind: operation
                .operation_kind
                .clone()
                .unwrap_or(OperationKind::Query),
            selections: SelectionTree::from_tree(&operation.selection_set),
            variables: operation.variable_definitions.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct Node {
    kind: Kind,
    /// The field's name, the fragment's type condition, or the spread's fragment name.
    name: Sym,
    children: Span,
    condition: Option<Condition>,
    attributes: Option<Arc<SelectionAttributes>>,
    arguments_hash: u64,
}

#[derive(Debug, Clone, Default)]
pub struct SelectionTree {
    strings: Interner,
    nodes: Vec<Node>,
    children: Vec<SelId>,
    root: Span,
}

impl SelectionTree {
    pub fn from_tree(selection_set: &SelectionSet) -> Self {
        let mut tree = SelectionTree::default();
        tree.root = tree.add_list(&selection_set.items);
        tree
    }

    /// Adds the items, then their list. A list's children go in before the list, so every
    /// list is one run in `children`.
    fn add_list(&mut self, items: &[SelectionItem]) -> Span {
        let ids: Vec<SelId> = items.iter().map(|item| self.add(item)).collect();
        let start = self.children.len() as u32;
        self.children.extend_from_slice(&ids);
        Span {
            start,
            len: ids.len() as u32,
        }
    }

    fn add(&mut self, item: &SelectionItem) -> SelId {
        let node = match item {
            SelectionItem::Field(field) => {
                let attributes = (field.alias.is_some() || field.arguments.is_some()).then(|| {
                    Arc::new(SelectionAttributes {
                        alias: field.alias.clone(),
                        arguments: field.arguments.clone(),
                    })
                });
                Node {
                    kind: Kind::Field,
                    name: self.strings.intern(&field.name),
                    children: self.add_list(&field.selections.items),
                    condition: field.into(),
                    attributes,
                    arguments_hash: field.arguments_hash(),
                }
            }
            SelectionItem::InlineFragment(fragment) => Node {
                kind: Kind::InlineFragment,
                name: self.strings.intern(&fragment.type_condition),
                children: self.add_list(&fragment.selections.items),
                condition: fragment.into(),
                attributes: None,
                arguments_hash: 0,
            },
            SelectionItem::FragmentSpread(name) => Node {
                kind: Kind::FragmentSpread,
                name: self.strings.intern(name),
                children: Span::EMPTY,
                condition: None,
                attributes: None,
                arguments_hash: 0,
            },
        };
        self.nodes.push(node);
        (self.nodes.len() - 1) as SelId
    }

    /// Adds a node; its children must be in already, see `add_children`.
    pub(super) fn add_node(
        &mut self,
        kind: Kind,
        name: &str,
        children: Span,
        condition: Option<Condition>,
        attributes: Option<SelectionAttributes>,
    ) -> SelId {
        let arguments_hash = attributes
            .as_ref()
            .and_then(|a| a.arguments.as_ref())
            .map_or(0, ArgumentsMap::hash_u64);
        let node = Node {
            kind,
            name: self.strings.intern(name),
            children,
            condition,
            attributes: attributes.map(Arc::new),
            arguments_hash,
        };
        self.nodes.push(node);
        (self.nodes.len() - 1) as SelId
    }

    /// A list of nodes added before, as one run.
    pub(super) fn add_children(&mut self, ids: &[SelId]) -> Span {
        let start = self.children.len() as u32;
        self.children.extend_from_slice(ids);
        Span {
            start,
            len: ids.len() as u32,
        }
    }

    pub(super) fn set_roots(&mut self, roots: Span) {
        self.root = roots;
    }

    /// Without the introspection fields, which the router answers itself: what
    /// `partition_operation` leaves for the subgraphs. A field or fragment left with nothing
    /// under it goes too.
    /// The selections of `list`, read the walker's way.
    pub fn from_arena(arena: &Arena, list: ListId) -> SelectionTree {
        fn add(tree: &mut SelectionTree, arena: &Arena, list: ListId) -> Span {
            let ids: Vec<SelId> = arena
                .items(list)
                .iter()
                .map(|&id| {
                    let node = arena.node(id);
                    let children = add(tree, arena, node.children);
                    let attributes = match node.kind {
                        Kind::Field => {
                            let alias = arena.opt_str(node.alias).map(str::to_string);
                            let arguments = arena.args(node.args).cloned();
                            (alias.is_some() || arguments.is_some())
                                .then_some(SelectionAttributes { alias, arguments })
                        }
                        _ => None,
                    };
                    let condition = match node.kind {
                        Kind::FragmentSpread => None,
                        _ => arena.condition(id),
                    };
                    tree.add_node(
                        node.kind,
                        arena.str(node.name),
                        children,
                        condition,
                        attributes,
                    )
                })
                .collect();
            tree.add_children(&ids)
        }
        let mut tree = SelectionTree::default();
        tree.root = add(&mut tree, arena, list);
        tree
    }

    pub fn without_introspection(&self) -> SelectionTree {
        fn keep(from: &SelectionTree, to: &mut SelectionTree, list: Span) -> Span {
            let mut ids = Vec::new();
            for &id in &from.children[list.range()] {
                let node = &from.nodes[id as usize];
                let name = from.strings.str(node.name);
                if node.kind == Kind::Field && name.starts_with("__") && name != "__typename" {
                    continue;
                }
                let children = keep(from, to, node.children);
                let empty = children.is_empty();
                let dropped = match node.kind {
                    Kind::Field => empty && !node.children.is_empty(),
                    Kind::InlineFragment => empty,
                    Kind::FragmentSpread => false,
                };
                if dropped {
                    continue;
                }
                to.nodes.push(Node {
                    name: to.strings.intern(name),
                    children,
                    ..node.clone()
                });
                ids.push((to.nodes.len() - 1) as SelId);
            }
            to.add_children(&ids)
        }
        let mut to = SelectionTree::default();
        to.root = keep(self, &mut to, self.root);
        to
    }

    /// `{ __typename }`
    pub fn typename() -> SelectionTree {
        let mut tree = SelectionTree::default();
        let typename = tree.add_node(Kind::Field, "__typename", Span::EMPTY, None, None);
        tree.root = tree.add_children(&[typename]);
        tree
    }

    /// Drops the roots `keep` says no to.
    pub fn retain_roots(&mut self, mut keep: impl FnMut(Sel<'_>) -> bool) {
        let ids: Vec<SelId> = self.children[self.root.range()]
            .iter()
            .copied()
            .filter(|&id| keep(Sel { tree: self, id }))
            .collect();
        self.root = self.add_children(&ids);
    }

    /// `... on <type> { <its selections> }` for each part, in one tree.
    pub fn fragments_of<'a>(
        parts: impl IntoIterator<Item = (&'a str, &'a SelectionTree)>,
    ) -> SelectionTree {
        let mut tree = SelectionTree::default();
        let ids: Vec<SelId> = parts
            .into_iter()
            .map(|(type_name, part)| {
                let children = tree.copy_list(part, part.root);
                tree.add_node(Kind::InlineFragment, type_name, children, None, None)
            })
            .collect();
        tree.root = tree.add_children(&ids);
        tree
    }

    /// `list` of `from`, and everything below it, copied in.
    fn copy_list(&mut self, from: &SelectionTree, list: Span) -> Span {
        let ids: Vec<SelId> = from.children[list.range()]
            .iter()
            .map(|&id| {
                let node = &from.nodes[id as usize];
                let children = self.copy_list(from, node.children);
                let name = self.strings.intern(from.strings.str(node.name));
                self.nodes.push(Node {
                    name,
                    children,
                    ..node.clone()
                });
                (self.nodes.len() - 1) as SelId
            })
            .collect();
        self.add_children(&ids)
    }

    /// What `SelectionSet::cost` gives for the same selections.
    pub fn cost(&self) -> u64 {
        1 + self.roots().map(|sel| sel.cost()).sum::<u64>()
    }

    pub fn roots(&self) -> impl Iterator<Item = Sel<'_>> + Clone {
        self.list(self.root)
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_empty()
    }

    fn list(&self, span: Span) -> impl Iterator<Item = Sel<'_>> + Clone {
        self.children[span.range()]
            .iter()
            .map(move |&id| Sel { tree: self, id })
    }
}

/// One selection of a `SelectionTree`.
#[derive(Clone, Copy)]
pub struct Sel<'a> {
    tree: &'a SelectionTree,
    id: SelId,
}

impl<'a> Sel<'a> {
    #[inline]
    fn node(&self) -> &'a Node {
        &self.tree.nodes[self.id as usize]
    }

    #[inline]
    pub fn kind(&self) -> Kind {
        self.node().kind
    }

    /// The field's name, the fragment's type condition, or the spread's fragment name.
    #[inline]
    pub fn name(&self) -> &'a str {
        self.tree.strings.str(self.node().name)
    }

    #[inline]
    pub fn is_leaf(&self) -> bool {
        self.node().children.is_empty()
    }

    #[inline]
    pub fn children(&self) -> impl Iterator<Item = Sel<'a>> + Clone {
        self.tree.list(self.node().children)
    }

    #[inline]
    pub fn condition(&self) -> Option<&'a Condition> {
        self.node().condition.as_ref()
    }

    #[inline]
    pub fn attributes(&self) -> Option<&'a Arc<SelectionAttributes>> {
        self.node().attributes.as_ref()
    }

    #[inline]
    pub fn arguments_hash(&self) -> u64 {
        self.node().arguments_hash
    }

    /// `SelectionItem::cost`
    pub fn cost(&self) -> u64 {
        1 + self.children().map(|sel| sel.cost()).sum::<u64>()
    }
}

/// Prints like the tree's `SelectionSet`: `{a b{c} ...on T @skip(if: $s){d}}`, nothing when empty.
impl std::fmt::Display for SelectionTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write_list(self.roots(), f)
    }
}

fn write_list<'a>(
    items: impl Iterator<Item = Sel<'a>> + Clone,
    f: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    if items.clone().next().is_none() {
        return Ok(());
    }
    f.write_str("{")?;
    for (i, sel) in items.enumerate() {
        if i > 0 {
            f.write_str(" ")?;
        }
        match sel.kind() {
            Kind::Field => {
                let attributes = sel.attributes();
                if let Some(alias) = attributes.and_then(|a| a.alias.as_deref()) {
                    write!(f, "{alias}: ")?;
                }
                f.write_str(sel.name())?;
                if let Some(args) = attributes
                    .and_then(|a| a.arguments.as_ref())
                    .filter(|a| !a.is_empty())
                {
                    write!(f, "({args})")?;
                }
            }
            Kind::InlineFragment => write!(f, "...on {}", sel.name())?,
            Kind::FragmentSpread => {
                write!(f, "...{}", sel.name())?;
                continue;
            }
        }
        if let Some(condition) = sel.condition() {
            write!(f, " {condition}")?;
        }
        write_list(sel.children(), f)?;
    }
    f.write_str("}")
}

/// Same selections, the way the tree's `SelectionSet` compares them.
impl PartialEq for SelectionTree {
    fn eq(&self, other: &Self) -> bool {
        fn same_list<'a, 'b>(
            a: impl Iterator<Item = Sel<'a>> + Clone,
            b: impl Iterator<Item = Sel<'b>> + Clone,
        ) -> bool {
            a.clone().count() == b.clone().count() && a.zip(b).all(|(a, b)| same(a, b))
        }
        fn same(a: Sel<'_>, b: Sel<'_>) -> bool {
            a.kind() == b.kind()
                && a.name() == b.name()
                && a.condition() == b.condition()
                && same_attributes(a.attributes(), b.attributes())
                && same_list(a.children(), b.children())
        }
        same_list(self.roots(), other.roots())
    }
}

impl Eq for SelectionTree {}

impl std::hash::Hash for SelectionTree {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        fn hash_list<'a, H: std::hash::Hasher>(
            items: impl Iterator<Item = Sel<'a>>,
            state: &mut H,
        ) {
            for sel in items {
                sel.name().hash(state);
                hash_list(sel.children(), state);
            }
        }
        hash_list(self.roots(), state);
    }
}

impl std::fmt::Display for Sel<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind() {
            Kind::Field => write!(f, "{}", self.name()),
            Kind::InlineFragment => write!(f, "... on {}", self.name()),
            Kind::FragmentSpread => write!(f, "...{}", self.name()),
        }
    }
}

impl std::fmt::Debug for Sel<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::parse_query;

    use super::*;
    use crate::query_planner::ast::selection_set::SelectionSet;

    #[test]
    fn mirrors_the_tree() {
        let doc = parse_query::<String>(
            "{ a: f(x: 1) { g ... on T @skip(if: $s) { h } } k(y: 2) z: f(x: 1) }",
        )
        .unwrap();
        let graphql_tools::parser::query::Definition::Operation(
            graphql_tools::parser::query::OperationDefinition::SelectionSet(set),
        ) = &doc.definitions[0]
        else {
            panic!()
        };
        let set: SelectionSet = set.clone().into();
        let tree = SelectionTree::from_tree(&set);

        let roots: Vec<_> = tree.roots().collect();
        assert_eq!(roots.len(), 3);
        assert_eq!(roots[0].name(), "f");
        assert_eq!(roots[0].attributes().unwrap().alias.as_deref(), Some("a"));
        let kids: Vec<_> = roots[0].children().collect();
        assert_eq!(kids[0].name(), "g");
        assert!(kids[0].is_leaf());
        assert_eq!(kids[1].kind(), Kind::InlineFragment);
        assert_eq!(kids[1].name(), "T");
        assert_eq!(kids[1].condition(), Some(&Condition::Skip("s".into())));
        assert_eq!(kids[1].children().next().unwrap().name(), "h");
        assert_eq!(roots[1].attributes().unwrap().alias, None);
        assert_ne!(roots[0].arguments_hash(), roots[1].arguments_hash());
        assert_eq!(roots[0].arguments_hash(), roots[2].arguments_hash());
        assert!(!same_attributes(
            roots[0].attributes(),
            roots[2].attributes()
        ));
    }
}
