use std::fmt::Display;

use super::flat::selection_tree::SelectionTree;

/// A federation field set (`@key`, `@requires`, a context's selection) on `type_name`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeAwareSelection<'a> {
    pub type_name: &'a str,
    pub selections: SelectionTree,
}

impl Display for TypeAwareSelection<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.selections.is_empty() {
            write!(f, "{{}}")
        } else {
            write!(f, "{}", self.selections)
        }
    }
}
