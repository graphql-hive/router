use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::{
    ast::selection_set::SelectionSet,
    utils::pretty_display::{get_indent, PrettyDisplay},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentDefinition {
    pub name: String,
    pub selection_set: SelectionSet,
    pub type_condition: String,
}

impl Ord for FragmentDefinition {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

impl PartialOrd for FragmentDefinition {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for FragmentDefinition {}

impl PartialEq for FragmentDefinition {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Display for FragmentDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "fragment {} on {} {}",
            self.name, self.type_condition, self.selection_set
        )?;

        Ok(())
    }
}

impl PrettyDisplay for FragmentDefinition {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);
        writeln!(
            f,
            "{indent}  fragment {} on {} {{",
            self.name, self.type_condition
        )?;
        self.selection_set.pretty_fmt(f, depth + 2)?;
        writeln!(f, "{indent}  }}")?;

        Ok(())
    }
}
