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
        write!(f, "{}", self.selection_set)
    }
}

impl PartialEq for TypeAwareSelection {
    fn eq(&self, other: &Self) -> bool {
        // TODO: This needs to be improved and check the internal selection sets correctly.
        self.type_name == other.type_name && self.selection_set.items == other.selection_set.items
    }
}

impl Eq for TypeAwareSelection {}

impl TypeAwareSelection {
    pub fn new(type_name: String, selection_set: SelectionSet) -> Self {
        Self {
            type_name,
            selection_set,
        }
    }

    pub fn cost(&self) -> u64 {
        let mut cost = 1;

        for node in &self.selection_set.items {
            cost += node.cost();
        }

        cost
    }
}
