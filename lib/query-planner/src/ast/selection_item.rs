use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use super::selection_set::{FieldSelection, FragmentSelection};

#[derive(Clone)]
pub enum SelectionItem {
    Field(FieldSelection),
    Fragment(FragmentSelection),
}

impl Hash for SelectionItem {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            SelectionItem::Field(field) => field.hash(state),
            SelectionItem::Fragment(fragment) => fragment.hash(state),
        }
    }
}

impl Display for SelectionItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Using the following instead of "field_selection.name" will print the full, nested selection set here
            // SelectionItem::Field(field_selection) => write!(f, "{}", field_selection),
            SelectionItem::Field(field_selection) => write!(f, "{}", field_selection.name),
            SelectionItem::Fragment(fragment_selection) => write!(f, "{}", fragment_selection),
        }
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
                SelectionItem::Fragment(FragmentSelection { type_name: a, .. }),
                SelectionItem::Fragment(FragmentSelection { type_name: b, .. }),
            ) => a.cmp(b),
            (
                SelectionItem::Field(FieldSelection { .. }),
                SelectionItem::Fragment(FragmentSelection { .. }),
            ) => std::cmp::Ordering::Less,
            (
                SelectionItem::Fragment(FragmentSelection { .. }),
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
            SelectionItem::Fragment(FragmentSelection { selections, .. }) => {
                Some(&selections.items)
            }
        }
    }

    pub fn sort_key(&self) -> String {
        match self {
            SelectionItem::Field(FieldSelection {
                name: field_name, ..
            }) => field_name.to_string(),
            SelectionItem::Fragment(FragmentSelection { type_name, .. }) => type_name.to_string(),
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
        matches!(self, SelectionItem::Fragment(_))
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
            SelectionItem::Fragment(FragmentSelection {
                type_name,
                selections,
            }) => f
                .debug_struct("SelectionItem::Fragment")
                .field("type_name", type_name)
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
            (SelectionItem::Fragment(f1), SelectionItem::Fragment(f2)) => {
                f1.type_name == f2.type_name && f1.selections.items == f2.selections.items
            }
            _ => false,
        }
    }
}

impl Eq for SelectionItem {}
