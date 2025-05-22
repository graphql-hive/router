use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use graphql_parser::query::{Selection as ParserSelection, SelectionSet as ParserSelectionSet};

use crate::utils::pretty_display::{get_indent, PrettyDisplay};

use super::{arguments::ArgumentsMap, selection_item::SelectionItem};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FieldSelection {
    pub name: String,
    pub selections: SelectionSet,
    pub alias: Option<String>,
    pub is_leaf: bool,
    pub arguments: ArgumentsMap,
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
            arguments: ArgumentsMap::default(),
        }
    }

    pub fn has_arguments(&self) -> bool {
        !self.arguments.is_empty()
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct InlineFragmentSelection {
    pub type_name: String,
    pub selections: SelectionSet,
}

impl Hash for InlineFragmentSelection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_name.hash(state);
        self.selections.hash(state);
    }
}

impl Display for FieldSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)?;

        if self.has_arguments() {
            write!(f, "({})", self.arguments)?;
        }

        write!(f, "{}", self.selections)
    }
}

impl PrettyDisplay for FieldSelection {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);
        if self.is_leaf {
            return writeln!(f, "{indent}{}", self.name);
        }

        writeln!(f, "{indent}{} {{", self.name)?;
        self.selections.pretty_fmt(f, depth + 1)?;
        writeln!(f, "{indent}}}")
    }
}

impl PrettyDisplay for SelectionSet {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        for item in self.items.iter() {
            item.pretty_fmt(f, depth)?;
        }

        Ok(())
    }
}

impl Display for InlineFragmentSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "... on {}", self.type_name)?;
        write!(f, "{}", self.selections)
    }
}

impl PrettyDisplay for InlineFragmentSelection {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);
        writeln!(f, "{indent}... on {} {{", self.type_name)?;
        self.selections.pretty_fmt(f, depth + 1)?;
        writeln!(f, "{indent}}}")
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
                arguments: (&field.arguments).into(),
            }),
            ParserSelection::InlineFragment(inline_fragment) => {
                SelectionItem::InlineFragment(InlineFragmentSelection {
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

#[cfg(test)]
mod tests {
    use crate::ast::value::Value;

    use super::*;

    #[test]
    fn print_simple_selection_set() {
        let selection_set = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "field1".to_string(),
                selections: SelectionSet::default(),
                alias: None,
                is_leaf: true,
                arguments: ArgumentsMap::default(),
            })],
        };

        insta::assert_snapshot!(
          selection_set,
          @"{field1}"
        )
    }

    #[test]
    fn selection_set_with_arguments() {
        let selection_set = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "field1".to_string(),
                selections: SelectionSet::default(),
                alias: None,
                is_leaf: true,
                arguments: vec![("id".to_string(), Value::Int(1))].into(),
            })],
        };

        insta::assert_snapshot!(
          selection_set,
          @"{field1(id: 1)}"
        )
    }

    #[test]
    fn complex_selection_set() {
        let selection_set = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "field1".to_string(),
                selections: SelectionSet::default(),
                alias: None,
                is_leaf: true,
                arguments: vec![
                    ("id".to_string(), Value::Int(1)),
                    ("name".to_string(), Value::String("test".to_string())),
                    (
                        "list".to_string(),
                        Value::List(vec![Value::Int(1), Value::Int(2)]),
                    ),
                    (
                        "obj".to_string(),
                        Value::Object(
                            vec![("key".to_string(), Value::String("value".to_string()))]
                                .into_iter()
                                .collect(),
                        ),
                    ),
                ]
                .into(),
            })],
        };

        insta::assert_snapshot!(
          selection_set,
          @r#"{field1(id: 1, list: [1, 2], name: "test", obj: {"key": "value"})}"#
        )
    }
}
