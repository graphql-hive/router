use std::{fmt::Display, hash::Hash};

use super::selection_set::SelectionSet;

#[derive(Debug, Clone)]
pub struct TypeAwareSelection {
    pub type_name: String,
    pub selection_set: SelectionSet,
}

impl Hash for TypeAwareSelection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_name.hash(state);
        self.selection_set.hash(state);
    }
}

impl Display for TypeAwareSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.selection_set.is_empty() {
            write!(f, "{{}}")
        } else {
            write!(f, "{}", self.selection_set)
        }
    }
}

impl PartialEq for TypeAwareSelection {
    fn eq(&self, other: &Self) -> bool {
        // TODO: This needs to be improved and check the internal selection sets correctly.
        self.type_name == other.type_name && self.selection_set.items == other.selection_set.items
    }
}

impl Eq for TypeAwareSelection {}
