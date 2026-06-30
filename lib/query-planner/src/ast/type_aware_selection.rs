use std::{fmt::Display, hash::Hash};

use super::selection_set::SelectionSet;

#[derive(Debug, Clone)]
pub struct TypeAwareSelection<'a> {
    pub type_name: &'a str,
    pub selection_set: SelectionSet,
}

impl Hash for TypeAwareSelection<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_name.hash(state);
        self.selection_set.hash(state);
    }
}

impl Display for TypeAwareSelection<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.selection_set.is_empty() {
            write!(f, "{{}}")
        } else {
            write!(f, "{}", self.selection_set)
        }
    }
}

impl PartialEq for TypeAwareSelection<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.type_name == other.type_name && self.selection_set.items == other.selection_set.items
    }
}

impl Eq for TypeAwareSelection<'_> {}
