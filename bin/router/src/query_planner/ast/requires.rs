//! Entity key selections used to build and hash subgraph representations.
//!
//! A fetch's `requires` lists the fields a subgraph needs to resolve an entity.
//! Execution reads it twice per entity: once to hash the entity and remove duplicates
//! (`Value::to_hash`) and once to write the representation that is sent
//! (`project_requires`).
//!
//! # Shape
//!
//! For `{ id ... on Product { upc } }`, the *root selections* are `id` and the inline fragment.
//! `upc` is inside the fragment. Execution starts at the roots and follows the children:
//!
//! ```text
//! root_selections() -> [ Field(id), InlineFragment(on Product) ]
//!                                     |
//!                                     +-- [ Field(upc) ]
//! ```
//!
//! # Storage
//!
//! Nodes live in one flat array. Each node's children are next to each other, and nodes use a
//! separate name table. Walking starts with [`RequiresSelectionSet::root_selections`] and uses
//! [`RequiresSelection`] to read each node. The indexes and name table stay inside this module.
//!
//! Execution, serialization, and printing share [`RequiresSelection`], a view of the stored data.
//!
//! The name table belongs to one `requires` value. Response keys and aliases come from the client,
//! so a shared table could grow forever if a client sent many names. These strings go away with the
//! plan-cache entry.
//!
//! The JSON must match `SelectionSet` byte for byte. This is the format sent by `lib/node-addon` to
//! Hive Gateway and returned by `hive-expose-query-plan`.

use std::{fmt, num::NonZeroU32};

use serde::{
    de::{Deserialize, Deserializer},
    ser::{SerializeSeq, Serializer},
    Serialize,
};

use crate::query_planner::{
    ast::{
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    },
    utils::pretty_display::{get_indent, PrettyDisplay},
};

#[derive(Clone, Debug, Default)]
pub struct RequiresSelectionSet {
    names: Box<[Box<str>]>,
    nodes: Box<[StoredSelection]>,
    root_len: u32,
}

#[derive(Clone, Copy)]
pub struct RequiresSelectionSetRef<'a> {
    owner: &'a RequiresSelectionSet,
    range: SelectionRange,
}

#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum RequiresSelection<'a> {
    Field {
        name: &'a str,
        #[serde(skip_serializing_if = "RequiresSelectionSetRef::is_empty")]
        selections: RequiresSelectionSetRef<'a>,
        #[serde(skip_serializing_if = "Option::is_none")]
        alias: Option<&'a str>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        omit_from_response: bool,
    },
    #[serde(rename_all = "camelCase")]
    InlineFragment {
        type_condition: &'a str,
        selections: RequiresSelectionSetRef<'a>,
        #[serde(skip_serializing_if = "Option::is_none")]
        skip_if: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        include_if: Option<&'a str>,
    },
    FragmentSpread(&'a str),
}

impl<'a> RequiresSelectionSetRef<'a> {
    pub fn is_empty(&self) -> bool {
        self.range.len == 0
    }

    pub fn len(&self) -> usize {
        self.range.len as usize
    }

    pub fn iter(&self) -> impl Iterator<Item = RequiresSelection<'a>> + 'a {
        let owner = self.owner;
        self.range.indices().map(move |i| owner.selection(i))
    }
}

impl RequiresSelectionSet {
    pub fn root_selections(&self) -> RequiresSelectionSetRef<'_> {
        self.selections(SelectionRange {
            start: 0,
            len: self.root_len,
        })
    }

    fn selections(&self, range: SelectionRange) -> RequiresSelectionSetRef<'_> {
        RequiresSelectionSetRef { owner: self, range }
    }

    fn opt_name(&self, id: Option<NameId>) -> Option<&str> {
        id.map(|id| self.name(id))
    }

    fn name(&self, id: NameId) -> &str {
        &self.names[id.index()]
    }

    fn selection(&self, i: usize) -> RequiresSelection<'_> {
        match &self.nodes[i] {
            StoredSelection::Field {
                name,
                alias,
                children,
                omit_from_response,
            } => RequiresSelection::Field {
                name: self.name(*name),
                selections: self.selections(*children),
                alias: self.opt_name(*alias),
                omit_from_response: *omit_from_response,
            },
            StoredSelection::InlineFragment {
                type_condition,
                skip_if,
                include_if,
                children,
            } => RequiresSelection::InlineFragment {
                type_condition: self.name(*type_condition),
                selections: self.selections(*children),
                skip_if: self.opt_name(*skip_if),
                include_if: self.opt_name(*include_if),
            },
            StoredSelection::FragmentSpread { name } => {
                RequiresSelection::FragmentSpread(self.name(*name))
            }
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NameId(NonZeroU32);

impl NameId {
    fn new(index: usize) -> Self {
        let raw = u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .and_then(NonZeroU32::new)
            .expect("name table outgrew u32");
        Self(raw)
    }

    fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct SelectionRange {
    start: u32,
    len: u32,
}

impl SelectionRange {
    fn indices(self) -> std::ops::Range<usize> {
        let start = self.start as usize;
        start..start + self.len as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StoredSelection {
    Field {
        name: NameId,
        alias: Option<NameId>,
        children: SelectionRange,
        omit_from_response: bool,
    },
    InlineFragment {
        type_condition: NameId,
        skip_if: Option<NameId>,
        include_if: Option<NameId>,
        children: SelectionRange,
    },
    FragmentSpread {
        name: NameId,
    },
}

impl StoredSelection {
    fn set_children(&mut self, range: SelectionRange) {
        match self {
            Self::Field { children, .. } | Self::InlineFragment { children, .. } => {
                *children = range;
            }
            Self::FragmentSpread { .. } => unreachable!("fragment spreads have no child list"),
        }
    }
}

impl From<&SelectionSet> for RequiresSelectionSet {
    fn from(set: &SelectionSet) -> Self {
        let mut builder = Builder::default();
        let roots = builder.append_siblings(&set.items);

        let mut cursor = 0;
        while let Some(&(node_index, items)) = builder.pending_children.get(cursor) {
            cursor += 1;
            let children = builder.append_siblings(items);
            builder.nodes[node_index].set_children(children);
        }

        Self {
            names: builder.names.into_boxed_slice(),
            nodes: builder.nodes.into_boxed_slice(),
            root_len: roots.len,
        }
    }
}

#[derive(Default)]
struct Builder<'a> {
    names: Vec<Box<str>>,
    nodes: Vec<StoredSelection>,
    pending_children: Vec<(usize, &'a [SelectionItem])>,
}

impl<'a> Builder<'a> {
    fn intern(&mut self, value: &str) -> NameId {
        if let Some(i) = self.names.iter().position(|n| &**n == value) {
            return NameId::new(i);
        }
        self.names.push(value.into());
        NameId::new(self.names.len() - 1)
    }

    fn intern_opt(&mut self, value: Option<&str>) -> Option<NameId> {
        value.map(|value| self.intern(value))
    }

    fn append_siblings(&mut self, items: &'a [SelectionItem]) -> SelectionRange {
        let start = u32::try_from(self.nodes.len()).expect("node array outgrew u32");
        for item in items {
            let node_index = self.nodes.len();
            match item {
                SelectionItem::Field(FieldSelection {
                    name,
                    selections,
                    alias,
                    omit_from_response,
                    ..
                }) => {
                    let name = self.intern(name);
                    let alias = self.intern_opt(alias.as_deref());
                    self.nodes.push(StoredSelection::Field {
                        name,
                        alias,
                        children: SelectionRange::default(),
                        omit_from_response: *omit_from_response,
                    });
                    if !selections.items.is_empty() {
                        self.pending_children.push((node_index, &selections.items));
                    }
                }
                SelectionItem::InlineFragment(InlineFragmentSelection {
                    type_condition,
                    selections,
                    skip_if,
                    include_if,
                }) => {
                    let type_condition = self.intern(type_condition);
                    let skip_if = self.intern_opt(skip_if.as_deref());
                    let include_if = self.intern_opt(include_if.as_deref());
                    self.nodes.push(StoredSelection::InlineFragment {
                        type_condition,
                        skip_if,
                        include_if,
                        children: SelectionRange::default(),
                    });
                    self.pending_children.push((node_index, &selections.items));
                }
                SelectionItem::FragmentSpread(name) => {
                    let name = self.intern(name);
                    self.nodes.push(StoredSelection::FragmentSpread { name });
                }
            }
        }
        let end = u32::try_from(self.nodes.len()).expect("node array outgrew u32");
        SelectionRange {
            start,
            len: end - start,
        }
    }
}

impl Serialize for RequiresSelectionSet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.root_selections().serialize(serializer)
    }
}

impl Serialize for RequiresSelectionSetRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for i in self.range.indices() {
            seq.serialize_element(&self.owner.selection(i))?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for RequiresSelectionSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let items = Vec::<SelectionItem>::deserialize(deserializer)?;
        Ok(Self::from(&SelectionSet { items }))
    }
}

impl PrettyDisplay for RequiresSelectionSet {
    fn pretty_fmt(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        self.pretty_slice(f, self.root_selections(), depth)
    }
}

impl RequiresSelectionSet {
    fn pretty_slice(
        &self,
        f: &mut fmt::Formatter<'_>,
        slice: RequiresSelectionSetRef<'_>,
        depth: usize,
    ) -> fmt::Result {
        let indent = get_indent(depth);
        for i in slice.range.indices() {
            match self.selection(i) {
                RequiresSelection::Field {
                    name,
                    selections,
                    alias,
                    ..
                } => {
                    if let Some(alias) = alias {
                        write!(f, "{indent}{alias}: ")?;
                    } else {
                        write!(f, "{indent}")?;
                    }
                    write!(f, "{name}")?;
                    if selections.is_empty() {
                        writeln!(f)?;
                        continue;
                    }
                    writeln!(f, " {{")?;
                    self.pretty_slice(f, selections, depth + 1)?;
                    writeln!(f, "{indent}}}")?;
                }
                RequiresSelection::InlineFragment {
                    type_condition,
                    selections,
                    skip_if,
                    include_if,
                } => {
                    write!(f, "{indent}... on {type_condition} ")?;
                    if let Some(skip_if) = skip_if {
                        write!(f, "@skip(if: ${skip_if}) ")?;
                    }
                    if let Some(include_if) = include_if {
                        write!(f, "@include(if: ${include_if}) ")?;
                    }
                    writeln!(f, "{{")?;
                    self.pretty_slice(f, selections, depth + 1)?;
                    writeln!(f, "{indent}}}")?;
                }
                RequiresSelection::FragmentSpread(name) => {
                    writeln!(f, "{indent}...{name}")?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::utils::parsing::parse_operation;
    use graphql_tools::parser::query;
    use std::fmt::Display;

    fn selection_set_from_str(requires: &str) -> SelectionSet {
        let operation = parse_operation(&format!("query {{ {requires} }}"));
        let selection_set = operation
            .definitions
            .into_iter()
            .find_map(|def| {
                let query::Definition::Operation(op) = def else {
                    return None;
                };
                match op {
                    query::OperationDefinition::SelectionSet(sel) => Some(sel),
                    query::OperationDefinition::Query(q) => Some(q.selection_set),
                    query::OperationDefinition::Mutation(m) => Some(m.selection_set),
                    query::OperationDefinition::Subscription(s) => Some(s.selection_set),
                }
            })
            .expect("operation must contain a selection set");
        selection_set.into()
    }

    /// Raw JSON, including key order. The output must match `SelectionSet`, so these tests compare
    /// the actual text instead of sorting keys first.
    fn json<T: Serialize>(value: &T) -> String {
        serde_json::to_string(value).expect("serializes")
    }

    const SHAPES: &[&str] = &[
        "id",
        "__typename id",
        "upc price { amount currency }",
        "... on Product { __typename upc }",
        "__typename ... on Product { upc dimensions { size weight } }",
        "alias: id",
        "a: id b: sku nested { c: inner }",
        "... on A { x } ... on B { y }",
        "__typename ... on Product { __typename }",
        "... on Product @skip(if: $s) { upc }",
        "... on Product @include(if: $i) { upc }",
        "... on Product @skip(if: $s) @include(if: $i) { upc }",
    ];

    #[test]
    fn serializes_identically_to_the_selection_set_it_replaces() {
        for source in SHAPES {
            let set = selection_set_from_str(source);
            let compact = RequiresSelectionSet::from(&set);
            assert_eq!(json(&set), json(&compact), "JSON diverged for `{source}`");
        }
    }

    /// The optimizer expands spreads before caching, so cached plans should not contain one. If a
    /// spread reaches serialization, it must fail instead of producing invalid `SelectionSet` JSON.
    #[test]
    fn serializing_a_fragment_spread_fails_the_way_the_selection_set_does() {
        let set = selection_set_from_str("id ...Foo");
        let compact = RequiresSelectionSet::from(&set);
        assert!(
            serde_json::to_string(&set).is_err(),
            "the type being matched must also refuse to serialize a spread"
        );
        assert!(serde_json::to_string(&compact).is_err());
    }

    #[test]
    fn omit_from_response_survives_lowering() {
        fn field(name: &str, omit: bool, selections: Vec<SelectionItem>) -> SelectionItem {
            SelectionItem::Field(FieldSelection {
                name: name.to_string(),
                selections: SelectionSet { items: selections },
                alias: None,
                arguments: None,
                skip_if: None,
                include_if: None,
                omit_from_response: omit,
            })
        }

        let set = SelectionSet {
            items: vec![
                field("omitted", true, vec![]),
                field("kept", false, vec![]),
                field(
                    "parent",
                    false,
                    vec![
                        field("omittedChild", true, vec![]),
                        field("keptChild", false, vec![]),
                    ],
                ),
                SelectionItem::InlineFragment(InlineFragmentSelection {
                    type_condition: "Product".to_string(),
                    selections: SelectionSet {
                        items: vec![field("omittedInFragment", true, vec![])],
                    },
                    skip_if: None,
                    include_if: None,
                }),
            ],
        };

        let compact = RequiresSelectionSet::from(&set);
        assert_eq!(
            json(&set),
            json(&compact),
            "JSON diverged for a programmatically built `omit_from_response` selection"
        );

        let json = serde_json::to_string(&compact).expect("serializes");
        assert_eq!(
            json.matches("\"omit_from_response\":true").count(),
            3,
            "expected exactly the three omitted fields to serialize the flag: {json}"
        );
    }

    #[test]
    fn pretty_prints_identically_to_the_selection_set_it_replaces() {
        struct Pretty<'a, T: PrettyDisplay>(&'a T, usize);
        impl<T: PrettyDisplay> Display for Pretty<'_, T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.pretty_fmt(f, self.1)
            }
        }

        for source in SHAPES {
            let set = selection_set_from_str(source);
            let compact = RequiresSelectionSet::from(&set);
            for depth in [0, 3] {
                assert_eq!(
                    Pretty(&set, depth).to_string(),
                    Pretty(&compact, depth).to_string(),
                    "pretty output diverged for `{source}` at depth {depth}"
                );
            }
        }
    }

    #[test]
    fn response_key_prefers_the_alias() {
        let compact = RequiresSelectionSet::from(&selection_set_from_str("alias: id plain"));
        let keys: Vec<_> = compact
            .root_selections()
            .iter()
            .map(|item| match item {
                RequiresSelection::Field { name, alias, .. } => {
                    (name.to_string(), alias.unwrap_or(name).to_string())
                }
                _ => panic!("expected fields"),
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                ("id".to_string(), "alias".to_string()),
                ("plain".to_string(), "plain".to_string()),
            ]
        );
    }

    #[test]
    fn nesting_survives_the_flattening() {
        let compact = RequiresSelectionSet::from(&selection_set_from_str(
            "a { b { c } d } e { f } ... on T { g { h } }",
        ));
        let mut shape = Vec::new();
        fn walk(slice: RequiresSelectionSetRef<'_>, depth: usize, out: &mut Vec<(usize, String)>) {
            for item in slice.iter() {
                match item {
                    RequiresSelection::Field {
                        name, selections, ..
                    } => {
                        out.push((depth, name.to_string()));
                        walk(selections, depth + 1, out);
                    }
                    RequiresSelection::InlineFragment {
                        type_condition,
                        selections,
                        ..
                    } => {
                        out.push((depth, format!("...on {type_condition}")));
                        walk(selections, depth + 1, out);
                    }
                    RequiresSelection::FragmentSpread(_) => out.push((depth, "...".to_string())),
                }
            }
        }
        walk(compact.root_selections(), 0, &mut shape);
        assert_eq!(
            shape,
            vec![
                (0, "a".into()),
                (1, "b".into()),
                (2, "c".into()),
                (1, "d".into()),
                (0, "e".into()),
                (1, "f".into()),
                (0, "...on T".into()),
                (1, "g".into()),
                (2, "h".into()),
            ]
        );
    }

    #[test]
    fn name_ids_check_the_index_boundary() {
        for index in [0, u32::MAX as usize - 1] {
            assert_eq!(NameId::new(index).index(), index);
        }
        for index in [u32::MAX as usize, usize::MAX] {
            assert!(std::panic::catch_unwind(|| NameId::new(index)).is_err());
        }
    }

    #[test]
    fn stays_small() {
        assert_eq!(std::mem::size_of::<Option<NameId>>(), 4);
        assert!(
            std::mem::size_of::<StoredSelection>() <= 24,
            "arena node grew to {} B; the point of this type is that it stays small",
            std::mem::size_of::<StoredSelection>()
        );
    }
}
