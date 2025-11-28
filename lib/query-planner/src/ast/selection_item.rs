use crate::{
    ast::normalization::utils::extract_type_condition,
    utils::pretty_display::{get_indent, PrettyDisplay},
};
use graphql_parser::query as query_ast;

use super::selection_set::{FieldSelection, InlineFragmentSelection};
use core::panic;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt::{Debug, Display},
    hash::Hash,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind")]
pub enum SelectionItem {
    Field(FieldSelection),
    InlineFragment(InlineFragmentSelection),
    FragmentSpread(String),
}

impl Hash for SelectionItem {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            SelectionItem::Field(field) => field.hash(state),
            SelectionItem::InlineFragment(fragment) => fragment.hash(state),
            SelectionItem::FragmentSpread(name) => name.hash(state),
        }
    }
}

impl Display for SelectionItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionItem::Field(field_selection) => write!(f, "{}", field_selection),
            SelectionItem::InlineFragment(fragment_selection) => {
                write!(f, "{}", fragment_selection)
            }
            SelectionItem::FragmentSpread(name) => write!(f, "...{}", name),
        }
    }
}

impl PrettyDisplay for SelectionItem {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        match self {
            SelectionItem::Field(field_selection) => field_selection.pretty_fmt(f, depth)?,
            SelectionItem::InlineFragment(fragment_selection) => {
                fragment_selection.pretty_fmt(f, depth)?
            }
            SelectionItem::FragmentSpread(name) => {
                let indent = get_indent(depth);
                write!(f, "{indent}...{}\n", name)?
            }
        }

        Ok(())
    }
}

impl Ord for SelectionItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (
                SelectionItem::Field(FieldSelection { .. }),
                SelectionItem::Field(FieldSelection { .. }),
            ) => self.sort_key().cmp(other.sort_key()),
            (
                SelectionItem::InlineFragment(InlineFragmentSelection {
                    type_condition: a, ..
                }),
                SelectionItem::InlineFragment(InlineFragmentSelection {
                    type_condition: b, ..
                }),
            ) => a.cmp(b),
            (
                SelectionItem::Field(FieldSelection { .. }),
                SelectionItem::InlineFragment(InlineFragmentSelection { .. }),
            ) => std::cmp::Ordering::Less,
            (
                SelectionItem::InlineFragment(InlineFragmentSelection { .. }),
                SelectionItem::Field(FieldSelection { .. }),
            ) => std::cmp::Ordering::Greater,
            (SelectionItem::FragmentSpread(a), SelectionItem::FragmentSpread(b)) => a.cmp(b),
            (SelectionItem::FragmentSpread(_), _) => std::cmp::Ordering::Less,
            (_, SelectionItem::FragmentSpread(_)) => std::cmp::Ordering::Greater,
        }
    }
}

impl PartialOrd for SelectionItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl SelectionItem {
    pub fn variable_usages(&self) -> BTreeSet<String> {
        match self {
            SelectionItem::Field(field_selection) => field_selection.variable_usages(),
            SelectionItem::InlineFragment(fragment_selection) => {
                fragment_selection.variable_usages()
            }
            SelectionItem::FragmentSpread(_fragment_spread) => BTreeSet::new(),
        }
    }

    pub fn selections(&self) -> Option<&Vec<SelectionItem>> {
        match self {
            SelectionItem::Field(FieldSelection { selections, .. }) => Some(&selections.items),
            SelectionItem::InlineFragment(InlineFragmentSelection { selections, .. }) => {
                Some(&selections.items)
            }
            SelectionItem::FragmentSpread(_fragment_spread) => None,
        }
    }

    pub fn sort_key(&self) -> &str {
        match self {
            SelectionItem::Field(field) => field.selection_identifier(),
            SelectionItem::InlineFragment(frag) => frag.type_condition.as_str(),
            SelectionItem::FragmentSpread(name) => name,
        }
    }

    pub fn cost(&self) -> u64 {
        let mut cost = 1;

        if let Some(child_selections) = self.selections() {
            for node in child_selections {
                cost += node.cost();
            }
        }

        cost
    }

    pub fn is_fragment(&self) -> bool {
        matches!(self, SelectionItem::InlineFragment(_))
    }

    pub fn is_field(&self) -> bool {
        matches!(self, SelectionItem::Field(_))
    }

    pub fn strip_for_plan_input(&self) -> Self {
        match self {
            SelectionItem::Field(field_selection) => SelectionItem::Field(FieldSelection {
                name: field_selection.name.clone(),
                selections: field_selection.selections.strip_for_plan_input(),
                alias: field_selection.alias.clone(),
                arguments: None,
                include_if: None,
                skip_if: None,
            }),
            SelectionItem::InlineFragment(fragment_selection) => {
                SelectionItem::InlineFragment(InlineFragmentSelection {
                    type_condition: fragment_selection.type_condition.clone(),
                    selections: fragment_selection.selections.strip_for_plan_input(),
                    skip_if: fragment_selection.skip_if.clone(),
                    include_if: fragment_selection.include_if.clone(),
                })
            }
            SelectionItem::FragmentSpread(name) => SelectionItem::FragmentSpread(name.clone()),
        }
    }
}

impl Debug for SelectionItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionItem::Field(FieldSelection {
                name, selections, ..
            }) => f
                .debug_struct("SelectionItem::Field")
                .field("name", name)
                .field("selections", selections)
                .finish(),
            SelectionItem::InlineFragment(InlineFragmentSelection {
                type_condition,
                selections,
                skip_if,
                include_if,
            }) => f
                .debug_struct("SelectionItem::Fragment")
                .field("type_name", type_condition)
                .field("selections", selections)
                .field("skip_if", skip_if)
                .field("include_if", include_if)
                .finish(),
            SelectionItem::FragmentSpread(name) => f
                .debug_struct("SelectionItem::FragmentSpread")
                .field("name", name)
                .finish(),
        }
    }
}

impl PartialEq for SelectionItem {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SelectionItem::Field(f1), SelectionItem::Field(f2)) => {
                if f1.name != f2.name {
                    return false;
                }

                f1.selections == f2.selections
            }
            (SelectionItem::InlineFragment(f1), SelectionItem::InlineFragment(f2)) => {
                f1.type_condition == f2.type_condition && f1.selections.items == f2.selections.items
            }
            _ => false,
        }
    }
}

impl Eq for SelectionItem {}

impl<'a, T: query_ast::Text<'a>> From<query_ast::Selection<'a, T>> for SelectionItem {
    fn from(value: query_ast::Selection<'a, T>) -> Self {
        match value {
            query_ast::Selection::Field(field) => SelectionItem::Field(field.into()),
            query_ast::Selection::InlineFragment(fragment) => {
                SelectionItem::InlineFragment(fragment.into())
            }
            query_ast::Selection::FragmentSpread(_) => {
                panic!("Received a fragment spread, but it should be inlined after normalization");
            }
        }
    }
}

#[derive(Default)]
struct ConditionsPair {
    skip_if: Option<String>,
    include_if: Option<String>,
}

impl<'a, T: query_ast::Text<'a>> From<&Vec<query_ast::Directive<'a, T>>> for ConditionsPair {
    fn from(directives: &Vec<query_ast::Directive<'a, T>>) -> Self {
        let mut skip_if: Option<String> = None;
        let mut include_if: Option<String> = None;

        for directive in directives {
            match directive.name.as_ref() {
                "skip" => {
                    let if_arg = directive.arguments.iter().find_map(|(name, value)| {
                        match name.as_ref() == "if" {
                            true => Some(value),
                            false => None,
                        }
                    });
                    if let Some(query_ast::Value::Variable(var_name)) = if_arg {
                        skip_if = Some(var_name.as_ref().to_string());
                    }
                }
                "include" => {
                    let if_arg = directive.arguments.iter().find_map(|(name, value)| {
                        match name.as_ref() == "if" {
                            true => Some(value),
                            false => None,
                        }
                    });
                    if let Some(query_ast::Value::Variable(var_name)) = if_arg {
                        include_if = Some(var_name.as_ref().to_string());
                    }
                }
                _ => {}
            }
        }

        Self {
            skip_if,
            include_if,
        }
    }
}

impl<'a, T: query_ast::Text<'a>> From<query_ast::Field<'a, T>> for FieldSelection {
    fn from(field: query_ast::Field<'a, T>) -> Self {
        let conditions: ConditionsPair = (&field.directives).into();

        Self {
            name: field.name.as_ref().to_string(),
            alias: field.alias.map(|alias| alias.as_ref().to_string()),
            arguments: match field.arguments.len() {
                0 => None,
                _ => Some(field.arguments.into()),
            },
            selections: field.selection_set.into(),
            skip_if: conditions.skip_if,
            include_if: conditions.include_if,
        }
    }
}

impl<'a, T: query_ast::Text<'a>> From<query_ast::InlineFragment<'a, T>>
    for InlineFragmentSelection
{
    fn from(value: query_ast::InlineFragment<'a, T>) -> Self {
        let conditions: ConditionsPair = (&value.directives).into();

        Self {
            type_condition: extract_type_condition(
                &value
                    .type_condition
                    .expect("expected a type condition after normalization"),
            )
            .to_string(),
            selections: value.selection_set.into(),
            skip_if: conditions.skip_if,
            include_if: conditions.include_if,
        }
    }
}
