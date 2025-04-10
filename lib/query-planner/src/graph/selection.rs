use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use graphql_parser_hive_fork::{
    parse_query,
    query::{Definition, OperationDefinition},
};

#[derive(Clone)]
pub enum SelectionNode {
    Field {
        field_name: String,
        type_name: String,
        selections: Option<Box<Vec<SelectionNode>>>,
    },
    Fragment {
        type_name: String,
        selections: Box<Vec<SelectionNode>>,
    },
}

impl SelectionNode {
    pub fn selections(&self) -> Option<&Vec<SelectionNode>> {
        match self {
            SelectionNode::Field { selections, .. } => selections.as_ref().map(|s| s.as_ref()),
            SelectionNode::Fragment { selections, .. } => Some(selections.as_ref()),
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
    type_name: String,
    key_fields_string: String,
    selection_set: Vec<SelectionNode>,
}

impl Selection {
    // pub fn parse_field_selection(
    //     selection_set_str: String,
    //     field_name: &str,
    //     type_name: &str,
    // ) -> Self {
    //     let parsed_doc = parse_query(&selection_set_str).unwrap().into_static();
    //     let parsed_definition = parsed_doc
    //         .definitions
    //         .first()
    //         .expect("failed to parse selection set")
    //         .clone();

    //     let selection_set = match parsed_definition {
    //         Definition::Operation(OperationDefinition::SelectionSet(selection_set)) => {
    //             Some(selection_set)
    //         }
    //         _ => None,
    //     }
    //     .expect("invalid selection set");

    //     SelectionNode::Field {
    //         selection_set,
    //         source: selection_set_str,
    //         field_name: field_name.to_string(),
    //         type_name: type_name.to_string(),
    //     }
    // }
}
