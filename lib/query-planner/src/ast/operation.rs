use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::{
    state::supergraph_state::OperationKind,
    utils::pretty_display::{get_indent, PrettyDisplay},
};

use super::{selection_item::SelectionItem, selection_set::SelectionSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDefinition {
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_kind: Option<OperationKind>,
    pub selection_set: SelectionSet,
    pub variable_definitions: Option<Vec<VariableDefinition>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubgraphFetchOperation(pub OperationDefinition);

impl SubgraphFetchOperation {
    pub fn get_inner_selection_set(&self) -> &SelectionSet {
        if let SelectionItem::Field(field) = &self.0.selection_set.items[0] {
            if field.name == "_entities" {
                return &field.selections;
            } else {
                return &self.0.selection_set;
            }
        }

        &self.0.selection_set
    }
}

impl Serialize for SubgraphFetchOperation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.0.to_string().as_str())
    }
}

impl PrettyDisplay for SubgraphFetchOperation {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);
        writeln!(f, "{indent}  {{")?;
        self.get_inner_selection_set().pretty_fmt(f, depth + 2)?;
        writeln!(f, "{indent}  }}")
    }
}

impl Display for OperationDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(operation_kind) = &self.operation_kind {
            write!(f, "{}", operation_kind)?;
        }

        if let Some(name) = &self.name {
            write!(f, "{}", name)?;
        }

        if let Some(variable_definitions) = &self.variable_definitions {
            if !variable_definitions.is_empty() {
                write!(f, "(")?;
                for (i, variable_definition) in variable_definitions.iter().enumerate() {
                    write!(f, "{}", variable_definition)?;

                    if i > 0 {
                        write!(f, ", ")?;
                    }
                }
                write!(f, ")")?;
            }
        }

        write!(f, "{}", self.selection_set)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TypeNode {
    List(Box<TypeNode>),
    NonNull(Box<TypeNode>),
    Named(String),
}

impl Display for TypeNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeNode::List(inner) => write!(f, "[{}]", inner),
            TypeNode::NonNull(inner) => write!(f, "{}!", inner),
            TypeNode::Named(name) => write!(f, "{}", name),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableDefinition {
    pub name: String,
    pub variable_type: TypeNode,
}

impl Display for VariableDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "${}:{}", self.name, self.variable_type)
    }
}
