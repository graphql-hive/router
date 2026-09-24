use std::{
    collections::BTreeMap,
    fmt::{self, Display},
};

use graphql_tools::parser::query::{Selection, SelectionSet};

use crate::query_planner::ast::normalization::utils::extract_type_condition;

/// What a `@provides` path makes available, as a tree of fields and type conditions.
///
/// The maps are sorted, so the same fields always give the same tree, no matter how the
/// `@provides` was written. That's what lets copies be keyed by it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Provided {
    pub fields: BTreeMap<String, Provided>,
    pub on_types: BTreeMap<String, Provided>,
}

impl Provided {
    pub fn from_selection_set(selection_set: &SelectionSet<'static, String>) -> Self {
        let mut provided = Self::default();
        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => {
                    // Every node resolves `__typename` already.
                    if field.name != "__typename" {
                        provided
                            .fields
                            .entry(field.name.clone())
                            .or_default()
                            .merge(Self::from_selection_set(&field.selection_set));
                    }
                }
                Selection::InlineFragment(fragment) => {
                    let type_name = extract_type_condition(
                        fragment
                            .type_condition
                            .as_ref()
                            .expect("Inline fragment without type condition detected"),
                    );
                    provided
                        .on_types
                        .entry(type_name.to_string())
                        .or_default()
                        .merge(Self::from_selection_set(&fragment.selection_set));
                }
                Selection::FragmentSpread(_) => {
                    // Fragment spreads should have been normalized (converted into inline fragments) at this point
                    panic!(
                        "Fragment spread detected. Expected either a Field or an Inline Fragment"
                    )
                }
            }
        }
        provided
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.on_types.is_empty()
    }

    pub fn merge(&mut self, other: Provided) {
        for (name, below) in other.fields {
            self.fields.entry(name).or_default().merge(below);
        }
        for (name, below) in other.on_types {
            self.on_types.entry(name).or_default().merge(below);
        }
    }
}

/// `{sku product{name} ...on Book{isbn}}`, used in node names.
impl Display for Provided {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{")?;
        let items = self
            .fields
            .iter()
            .map(|(name, below)| (None, name, below))
            .chain(
                self.on_types
                    .iter()
                    .map(|(name, below)| (Some("...on "), name, below)),
            );
        for (i, (prefix, name, below)) in items.enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
            write!(f, "{}{}", prefix.unwrap_or_default(), name)?;
            if !below.is_empty() {
                write!(f, "{}", below)?;
            }
        }
        write!(f, "}}")
    }
}
