use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use graphql_parser_hive_fork::{
    parse_query,
    query::{Definition, OperationDefinition},
};

#[derive(Clone, Debug)]
pub struct SelectionNodeField {
    pub field_name: String,
    pub type_name: String,
    pub selections: Option<Vec<SelectionNode>>,
}

impl Hash for SelectionNodeField {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.field_name.hash(state);
        self.type_name.hash(state);
        self.selections.hash(state);
    }
}

impl SelectionNodeField {
    pub fn is_leaf(&self) -> bool {
        match &self.selections {
            Some(selections) => selections.is_empty(),
            None => true,
        }
    }
}

#[derive(Clone)]
pub struct SelectionNodeFragment {
    pub type_name: String,
    pub selections: Vec<SelectionNode>,
}

impl Hash for SelectionNodeFragment {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_name.hash(state);
        self.selections.hash(state);
    }
}

#[derive(Clone)]
pub enum SelectionNode {
    Field(SelectionNodeField),
    Fragment(SelectionNodeFragment),
}

impl Hash for SelectionNode {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            SelectionNode::Field(field) => field.hash(state),
            SelectionNode::Fragment(fragment) => fragment.hash(state),
        }
    }
}

impl Display for SelectionNodeField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.field_name)?;

        if let Some(child_selections) = &self.selections {
            if !child_selections.is_empty() {
                write!(f, " {{")?;

                for selection in child_selections {
                    write!(f, "{}", selection)?;
                }

                write!(f, " }}")?;
            }
        }

        Ok(())
    }
}

impl Display for SelectionNodeFragment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "... {}", self.type_name)?;

        if !self.selections.is_empty() {
            write!(f, " {{")?;

            for selection in self.selections.iter() {
                write!(f, "{}", selection)?;
            }

            write!(f, " }}")?;
        }

        Ok(())
    }
}

impl Display for SelectionNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionNode::Field(field_selection) => write!(f, "{}", field_selection),
            SelectionNode::Fragment(fragment_selection) => write!(f, "{}", fragment_selection),
        }?;

        Ok(())
    }
}

impl Ord for SelectionNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (
                SelectionNode::Field(SelectionNodeField { .. }),
                SelectionNode::Field(SelectionNodeField { .. }),
            ) => self.sort_key().cmp(&other.sort_key()),
            (
                SelectionNode::Fragment(SelectionNodeFragment { type_name: a, .. }),
                SelectionNode::Fragment(SelectionNodeFragment { type_name: b, .. }),
            ) => a.cmp(b),
            (
                SelectionNode::Field(SelectionNodeField { .. }),
                SelectionNode::Fragment(SelectionNodeFragment { .. }),
            ) => std::cmp::Ordering::Less,
            (
                SelectionNode::Fragment(SelectionNodeFragment { .. }),
                SelectionNode::Field(SelectionNodeField { .. }),
            ) => std::cmp::Ordering::Greater,
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
            SelectionNode::Field(SelectionNodeField { selections, .. }) => selections.as_ref(),
            SelectionNode::Fragment(SelectionNodeFragment { selections, .. }) => Some(selections),
        }
    }

    pub fn sort_key(&self) -> String {
        match self {
            SelectionNode::Field(SelectionNodeField {
                field_name,
                type_name,
                ..
            }) => format!("{}.{}", type_name, field_name),
            SelectionNode::Fragment(SelectionNodeFragment { type_name, .. }) => {
                type_name.to_string()
            }
        }
    }

    pub fn type_name(&self) -> &str {
        match self {
            SelectionNode::Field(SelectionNodeField { type_name, .. }) => type_name,
            SelectionNode::Fragment(SelectionNodeFragment { type_name, .. }) => type_name,
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
}

impl Debug for SelectionNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionNode::Field(SelectionNodeField {
                field_name,
                type_name,
                selections,
            }) => f
                .debug_struct("PlanSelection::Field")
                .field("field_name", field_name)
                .field("type_name", type_name)
                .field("selections", selections)
                .finish(),
            SelectionNode::Fragment(SelectionNodeFragment {
                type_name,
                selections,
            }) => f
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
                SelectionNode::Field(SelectionNodeField {
                    field_name,
                    type_name,
                    ..
                }),
                SelectionNode::Field(SelectionNodeField {
                    field_name: other_field_name,
                    type_name: other_type_name,
                    ..
                }),
            ) => field_name == other_field_name && type_name == other_type_name,
            (
                SelectionNode::Fragment(SelectionNodeFragment {
                    type_name,
                    selections,
                    ..
                }),
                SelectionNode::Fragment(SelectionNodeFragment {
                    type_name: other_type_name,
                    selections: other_selections,
                    ..
                }),
            ) => type_name == other_type_name && selections == other_selections,
            _ => false,
        }
    }
}

impl Eq for SelectionNode {}

#[derive(Debug, Clone)]
pub struct Selection {
    pub type_name: String,
    pub key_fields_string: String,
    pub selection_set: Vec<SelectionNode>,
}

impl Hash for Selection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_name.hash(state);
        self.key_fields_string.hash(state);
        self.selection_set.hash(state);
    }
}

impl Display for Selection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{{}}}", self.key_fields_string)
    }
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        // TODO: This needs to be improved and check the internal selection sets correctly.
        self.type_name == other.type_name
            && self.key_fields_string == other.key_fields_string
            && self.selection_set == other.selection_set
    }
}

impl Eq for Selection {}

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

    pub fn cost(&self) -> u64 {
        let mut cost = 1;

        for node in &self.selection_set {
            cost += node.cost();
        }

        cost
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
