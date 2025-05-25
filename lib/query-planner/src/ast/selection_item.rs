use crate::utils::pretty_display::PrettyDisplay;

use super::selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet};
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind")]
pub enum SelectionItem {
    Field(FieldSelection),
    InlineFragment(InlineFragmentSelection),
}

impl Hash for SelectionItem {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            SelectionItem::Field(field) => field.hash(state),
            SelectionItem::InlineFragment(fragment) => fragment.hash(state),
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
            ) => self.sort_key().cmp(&other.sort_key()),
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
        }
    }
}

impl PartialOrd for SelectionItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl SelectionItem {
    pub fn selections(&self) -> Option<&Vec<SelectionItem>> {
        match self {
            SelectionItem::Field(FieldSelection { selections, .. }) => Some(&selections.items),
            SelectionItem::InlineFragment(InlineFragmentSelection { selections, .. }) => {
                Some(&selections.items)
            }
        }
    }

    pub fn selection_set(&self) -> &SelectionSet {
        match self {
            SelectionItem::Field(FieldSelection { selections, .. }) => selections,
            SelectionItem::InlineFragment(InlineFragmentSelection { selections, .. }) => selections,
        }
    }

    pub fn sort_key(&self) -> String {
        match self {
            SelectionItem::Field(FieldSelection {
                name: field_name, ..
            }) => field_name.to_string(),
            SelectionItem::InlineFragment(InlineFragmentSelection { type_condition, .. }) => {
                type_condition.to_string()
            }
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
            }) => f
                .debug_struct("SelectionItem::Fragment")
                .field("type_name", type_condition)
                .field("selections", selections)
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
