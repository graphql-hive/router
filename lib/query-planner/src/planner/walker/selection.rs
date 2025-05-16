use std::fmt::Display;

use graphql_parser_hive_fork::query::{
    Selection as ParserSelection, SelectionSet as ParserSelectionSet,
};

#[derive(Debug, Clone)]
pub struct SelectionSet {
    pub items: Vec<SelectionItem>,
}

#[derive(Debug, Clone)]
pub struct FieldSelection {
    pub name: String,
    pub alias: Option<String>,
    pub is_leaf: bool,
    pub selections: SelectionSet,
}

#[derive(Debug, Clone)]
pub struct FragmentSelection {
    pub type_selection: Option<String>,
    pub selections: SelectionSet,
}

#[derive(Debug, Clone)]
pub enum SelectionItem {
    Field(FieldSelection),
    Fragment(FragmentSelection),
}

impl SelectionItem {
    pub fn is_fragment(&self) -> bool {
        matches!(self, SelectionItem::Fragment(_))
    }

    pub fn is_leaf(&self) -> bool {
        match self {
            SelectionItem::Field(field) => field.is_leaf,
            SelectionItem::Fragment(_) => false,
        }
    }

    pub fn is_field(&self) -> bool {
        matches!(self, SelectionItem::Field(_))
    }
}

impl PartialEq for SelectionItem {
    fn eq(&self, other: &SelectionItem) -> bool {
        match (self, other) {
            (SelectionItem::Field(self_field), SelectionItem::Field(other_field)) => {
                if self_field.name != other_field.name {
                    return false;
                }

                return self_field.selections == other_field.selections;
            }
            // TODO: compare fragments too
            _ => false,
        }
    }
}

impl PartialEq for SelectionSet {
    fn eq(&self, other: &SelectionSet) -> bool {
        if self.items.len() != other.items.len() {
            return false;
        }

        self.items
            .iter()
            .all(|self_item| other.items.iter().any(|other_item| self_item == other_item))
    }
}

impl Display for SelectionSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, item) in self.items.iter().enumerate() {
            if i + 1 == self.items.len() {
                write!(f, "{}", item)?;
            } else {
                write!(f, "{} ", item)?;
            }
        }
        write!(f, "}}")?;
        Ok(())
    }
}

impl Display for SelectionItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionItem::Field(field) => write!(f, "{}", field.name),
            SelectionItem::Fragment(fragment) => match &fragment.type_selection {
                Some(type_selection) => write!(f, "... on {}", type_selection),
                None => write!(f, "..."),
            },
        }
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
                    type_selection: inline_fragment
                        .type_condition
                        .as_ref()
                        .map(|t| t.to_string()),
                    selections: (&inline_fragment.selection_set).into(),
                })
            }
            ParserSelection::FragmentSpread(_) => {
                unimplemented!("FragmentSpread is not supported")
            }
        }
    }
}
