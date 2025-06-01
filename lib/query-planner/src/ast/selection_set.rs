use serde::{ser::SerializeSeq, Deserialize, Serialize};
use std::{
    collections::{BTreeSet},
    fmt::{Debug, Display},
    hash::Hash,
};

use crate::utils::pretty_display::{get_indent, PrettyDisplay};

use super::{arguments::ArgumentsMap, selection_item::SelectionItem};

#[derive(Debug, Clone, Default, Deserialize)]
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

    pub fn variable_usages(&self) -> BTreeSet<String> {
        self.items
            .iter()
            .flat_map(|item| item.variable_usages())
            .collect()
    }
}

impl Hash for SelectionSet {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.items.hash(state);
    }
}

impl Serialize for SelectionSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.items.len()))?;
        for e in &self.items {
            seq.serialize_element(&e)?;
        }
        seq.end()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FieldSelection {
    pub name: String,
    #[serde(skip_serializing_if = "SelectionSet::is_empty")]
    pub selections: SelectionSet,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<ArgumentsMap>,
}

impl Hash for FieldSelection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.selections.hash(state);
    }
}

impl PartialEq for FieldSelection {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.alias == other.alias
            && self.arguments() == other.arguments()
    }
}

impl FieldSelection {
    pub fn is_leaf(&self) -> bool {
        self.selections.is_empty()
    }

    pub fn new_typename() -> Self {
        FieldSelection {
            name: "__typename".to_string(),
            alias: None,
            selections: SelectionSet::default(),
            arguments: None,
        }
    }

    pub fn variable_usages(&self) -> BTreeSet<String> {
        let mut usages = BTreeSet::new();

        if let Some(arguments) = &self.arguments {
            for value in arguments.values() {
                usages.extend(value.variable_usages());
            }
        }

        usages.extend(self.selections.variable_usages());
        usages
    }

    pub fn arguments(&self) -> Option<&ArgumentsMap> {
        match &self.arguments {
            Some(arguments) => {
                if arguments.is_empty() {
                    None
                } else {
                    Some(arguments)
                }
            }
            None => None,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineFragmentSelection {
    pub type_condition: String,
    pub selections: SelectionSet,
}

impl Hash for InlineFragmentSelection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_condition.hash(state);
        self.selections.hash(state);
    }
}

impl Display for FieldSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(alias) = &self.alias {
            write!(f, "{}: ", alias)?;
        }

        write!(f, "{}", self.name)?;

        if let Some(arguments) = &self.arguments() {
            write!(f, "({})", arguments)?;
        }

        write!(f, "{}", self.selections)
    }
}

impl PrettyDisplay for FieldSelection {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);

        let alias_str = match &self.alias {
            Some(alias_name) => format!("{}: ", alias_name),
            None => String::new(),
        };

        let args_str = match &self.arguments() {
            Some(arguments) => format!("({})", arguments),
            None => String::new(),
        };

        if self.is_leaf() {
            return writeln!(f, "{indent}{}{}{}", alias_str, self.name, args_str);
        }

        writeln!(f, "{indent}{}{}{} {{", alias_str, self.name, args_str)?;
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
        write!(f, "...on {}", self.type_condition)?;
        write!(f, "{}", self.selections)
    }
}

impl PrettyDisplay for InlineFragmentSelection {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);
        writeln!(f, "{indent}... on {} {{", self.type_condition)?;
        self.selections.pretty_fmt(f, depth + 1)?;
        writeln!(f, "{indent}}}")
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::value::Value;

    use super::*;

    #[test]
    fn print_alias_selection_set() {
        let selection_set = SelectionSet {
            items: vec![
                SelectionItem::Field(FieldSelection {
                    name: "field1".to_string(),
                    selections: SelectionSet::default(),
                    alias: Some("f".to_string()),
                    arguments: None,
                }),
                SelectionItem::Field(FieldSelection {
                    name: "field2".to_string(),
                    selections: SelectionSet {
                        items: vec![SelectionItem::Field(FieldSelection {
                            name: "nested".to_string(),
                            selections: SelectionSet::default(),
                            alias: Some("n".to_string()),
                            arguments: Some(("a".to_string(), Value::Int(1)).into()),
                        })],
                    },
                    alias: Some("f2".to_string()),
                    arguments: None,
                }),
            ],
        };

        insta::assert_snapshot!(
          selection_set,
          @"{f: field1 f2: field2{n: nested(a: 1)}}"
        )
    }

    #[test]
    fn print_simple_selection_set() {
        let selection_set = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "field1".to_string(),
                selections: SelectionSet::default(),
                alias: None,
                arguments: None,
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
                arguments: Some(vec![("id".to_string(), Value::Int(1))].into()),
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
                arguments: Some(
                    vec![
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
                ),
            })],
        };

        insta::assert_snapshot!(
          selection_set,
          @r#"{field1(id: 1, list: [1, 2], name: "test", obj: {"key": "value"})}"#
        )
    }
}
