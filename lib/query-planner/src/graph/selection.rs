use std::{fmt::Debug, hash::Hash};

use graphql_parser_hive_fork::{
    parse_query,
    query::{Definition, OperationDefinition, SelectionSet},
};

#[derive(Clone)]
pub struct GraphSelection {
    pub selection_set: SelectionSet<'static, String>,
    pub source: String,
}

impl Debug for GraphSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("")
            .field("selection_set", &self.source)
            .finish()
    }
}

impl PartialEq for GraphSelection {
    fn eq(&self, other: &Self) -> bool {
        &self.selection_set == &other.selection_set || self.source == other.source
    }
}

impl Eq for GraphSelection {}

impl Hash for GraphSelection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.source.hash(state);
    }
}

impl GraphSelection {
    // Modified to take source_str with correct lifetime
    pub fn from_selection_set(
        selection_set: &SelectionSet<'static, String>,
        source_str: &str,
    ) -> Self {
        GraphSelection {
            selection_set: selection_set.clone(),
            source: source_str.to_string(),
        }
    }

    pub fn parse(selection_set_str: String) -> Self {
        let parsed_doc = parse_query(&selection_set_str).unwrap().into_static();
        let parsed_definition = parsed_doc
            .definitions
            .first()
            .expect("failed to parse selection set")
            .clone();

        let selection_set = match parsed_definition {
            Definition::Operation(OperationDefinition::SelectionSet(selection_set)) => {
                Some(selection_set)
            }
            _ => None,
        }
        .expect("invalid selection set");

        GraphSelection {
            selection_set,
            source: selection_set_str,
        }
    }
}
