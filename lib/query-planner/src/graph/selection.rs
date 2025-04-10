use std::fmt::Debug;

use graphql_parser_hive_fork::{
    parse_query,
    query::{Definition, OperationDefinition},
};

#[derive(Clone)]
pub enum SelectionNode {
    Field {
        field_name: String,
        type_name: String,
        selections: Option<Vec<SelectionNode>>,
    },
    Fragment {
        type_name: String,
        selections: Vec<SelectionNode>,
    },
}

impl Ord for SelectionNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (SelectionNode::Field { .. }, SelectionNode::Field { .. }) => {
                self.sort_key().cmp(&other.sort_key())
            }
            (
                SelectionNode::Fragment { type_name: a, .. },
                SelectionNode::Fragment { type_name: b, .. },
            ) => a.cmp(b),
            (SelectionNode::Field { .. }, SelectionNode::Fragment { .. }) => {
                std::cmp::Ordering::Less
            }
            (SelectionNode::Fragment { .. }, SelectionNode::Field { .. }) => {
                std::cmp::Ordering::Greater
            }
        }
    }
}

impl PartialOrd for SelectionNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl SelectionNode {
    pub fn selections(&self) -> Option<&Vec<SelectionNode>> {
        match self {
            SelectionNode::Field { selections, .. } => selections.as_ref(),
            SelectionNode::Fragment { selections, .. } => Some(selections),
        }
    }

    pub fn sort_key(&self) -> String {
        match self {
            SelectionNode::Field {
                field_name,
                type_name,
                ..
            } => format!("{}.{}", type_name, field_name),
            SelectionNode::Fragment { type_name, .. } => type_name.to_string(),
        }
    }

    pub fn type_name(&self) -> &str {
        match self {
            SelectionNode::Field { type_name, .. } => type_name,
            SelectionNode::Fragment { type_name, .. } => type_name,
        }
    }
}

impl Debug for SelectionNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionNode::Field {
                field_name,
                type_name,
                selections,
            } => f
                .debug_struct("PlanSelection::Field")
                .field("field_name", field_name)
                .field("type_name", type_name)
                .field("selections", selections)
                .finish(),
            SelectionNode::Fragment {
                type_name,
                selections,
            } => f
                .debug_struct("PlanSelection::Fragment")
                .field("type_name", type_name)
                .field("selections", selections)
                .finish(),
        }
    }
}

impl PartialEq for SelectionNode {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                SelectionNode::Field {
                    field_name,
                    type_name,
                    ..
                },
                SelectionNode::Field {
                    field_name: other_field_name,
                    type_name: other_type_name,
                    ..
                },
            ) => field_name == other_field_name && type_name == other_type_name,
            (
                SelectionNode::Fragment {
                    type_name,
                    selections,
                    ..
                },
                SelectionNode::Fragment {
                    type_name: other_type_name,
                    selections: other_selections,
                    ..
                },
            ) => type_name == other_type_name && selections == other_selections,
            _ => false,
        }
    }
}

impl Eq for SelectionNode {}

// impl Hash for SelectionNode {
//     fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
//         self.source().hash(state);
//     }
// }

pub struct Selection {
    pub type_name: String,
    pub key_fields_string: String,
    pub selection_set: Vec<SelectionNode>,
}

impl Selection {
    pub fn new(
        type_name: String,
        key_fields_string: String,
        selection_set: Vec<SelectionNode>,
    ) -> Self {
        Self {
            type_name,
            key_fields_string,
            selection_set,
        }
    }
}

pub fn parse_selection_set(
    selection_set_str: &str,
) -> graphql_parser_hive_fork::query::SelectionSet<'static, String> {
    let parsed_doc = parse_query(selection_set_str).unwrap().into_static();
    let parsed_definition = parsed_doc
        .definitions
        .first()
        .expect("failed to parse selection set")
        .clone();

    match parsed_definition {
        Definition::Operation(OperationDefinition::SelectionSet(selection_set)) => {
            Some(selection_set)
        }
        _ => None,
    }
    .expect("invalid selection set")
}
