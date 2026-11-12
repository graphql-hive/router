//! A normalized selection set in flat arrays, read-only, the way the planner's walker reads it.
//!
//! A selection is an index. A field's alias and arguments sit behind one `Arc`, so a path that
//! goes through the field shares them instead of copying a map of arguments on every step.

use std::sync::Arc;

use super::{Interner, Kind, SelId, Span, Sym};
use crate::query_planner::ast::{
    arguments::ArgumentsMap, merge_path::Condition, selection_item::SelectionItem,
    selection_set::SelectionSet,
};

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
