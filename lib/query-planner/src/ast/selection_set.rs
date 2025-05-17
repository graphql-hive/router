use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use graphql_parser_hive_fork::query::{
    Selection as ParserSelection, SelectionSet as ParserSelectionSet,
};

use super::selection_item::SelectionItem;

#[derive(Debug, Clone, Default)]
pub struct SelectionSet {
    pub items: Vec<SelectionItem>,
}

impl PartialEq for SelectionSet {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl Eq for SelectionSet {}

impl Display for SelectionSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.items.is_empty() {
            return Ok(());
        }

        write!(f, "{{")?;

        write!(
            f,
            "{}",
            self.items
                .iter()
                .map(|v| format!("{}", v))
                .collect::<Vec<_>>()
                .join(" ")
        )?;

        write!(f, "}}")
    }
}

impl SelectionSet {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Hash for SelectionSet {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.items.hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct FieldSelection {
    pub name: String,
    pub selections: SelectionSet,
    pub alias: Option<String>,
    pub is_leaf: bool,
}

impl Hash for FieldSelection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.selections.hash(state);
    }
}

impl FieldSelection {
    pub fn is_leaf(&self) -> bool {
        self.is_leaf
    }

    pub fn new_typename() -> Self {
        FieldSelection {
            name: "__typename".to_string(),
            alias: None,
            is_leaf: true,
            selections: SelectionSet::default(),
        }
    }
}

#[derive(Clone)]
pub struct FragmentSelection {
    pub type_name: String,
    pub selections: SelectionSet,
}

impl Hash for FragmentSelection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_name.hash(state);
        self.selections.hash(state);
    }
}

impl Display for FieldSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)?;
        write!(f, "{}", self.selections)
    }
}

impl Display for FragmentSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "... on {}", self.type_name)?;
        write!(f, "{}", self.selections)
    }
}

impl From<&ParserSelectionSet<'_, String>> for SelectionSet {
    fn from(parser_selection_set: &ParserSelectionSet<'_, String>) -> Self {
        SelectionSet {
            items: parser_selection_set
                .items
                .iter()
                .map(|parser_selection_item| parser_selection_item.into())
                .collect(),
        }
    }
}

impl From<&ParserSelection<'_, String>> for SelectionItem {
    fn from(parser_selection: &ParserSelection<'_, String>) -> Self {
        match parser_selection {
            ParserSelection::Field(field) => SelectionItem::Field(FieldSelection {
                name: field.name.to_string(),
                alias: field.alias.as_ref().map(|alias| alias.to_string()),
                is_leaf: field.selection_set.items.is_empty(),
                selections: (&field.selection_set).into(),
            }),
            ParserSelection::InlineFragment(inline_fragment) => {
                SelectionItem::Fragment(FragmentSelection {
                    type_name: inline_fragment
                        .type_condition
                        .as_ref()
                        .map(|t| t.to_string())
                        .unwrap(),
                    selections: (&inline_fragment.selection_set).into(),
                })
            }
            ParserSelection::FragmentSpread(_) => {
                unimplemented!("FragmentSpread is not supported")
            }
        }
    }
}
