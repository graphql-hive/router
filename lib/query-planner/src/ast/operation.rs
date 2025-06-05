use std::fmt::Display;

use crate::{ast::hash::ast_hash, state::supergraph_state::TypeNode};
use graphql_parser::query as parser;
use serde::{Deserialize, Serialize};

use crate::{
    state::supergraph_state::OperationKind,
    utils::pretty_display::{get_indent, PrettyDisplay},
};

use super::{selection_item::SelectionItem, selection_set::SelectionSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDefinition {
    pub name: Option<String>,
    // TODO: Should operation_kind be OperationKind or Option<OperationKind>?
    // I don't see a scenario where it should be set to None?
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_kind: Option<OperationKind>,
    pub selection_set: SelectionSet,
    pub variable_definitions: Option<Vec<VariableDefinition>>,
}

impl OperationDefinition {
    pub fn parts(&self) -> (&OperationKind, &SelectionSet) {
        (
            self.operation_kind
                .as_ref()
                .unwrap_or(&OperationKind::Query),
            &self.selection_set,
        )
    }
    pub fn hash(&self) -> u64 {
        ast_hash(self)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubgraphFetchOperation {
    pub operation_def: OperationDefinition,
    pub operation_str: String,
}

impl SubgraphFetchOperation {
    pub fn get_inner_selection_set(&self) -> &SelectionSet {
        if let SelectionItem::Field(field) = &self.operation_def.selection_set.items[0] {
            if field.name == "_entities" {
                return &field.selections;
            } else {
                return &self.operation_def.selection_set;
            }
        }

        &self.operation_def.selection_set
    }
}

impl Serialize for SubgraphFetchOperation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.operation_def.to_string())
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
            write!(f, " {} ", name)?;
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
pub struct VariableDefinition {
    pub name: String,
    pub variable_type: TypeNode,
    pub default_value: Option<crate::ast::value::Value>,
}

impl Display for VariableDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.default_value {
            Some(default_value) => {
                write!(f, "${}:{}={}", self.name, self.variable_type, default_value)
            }
            None => write!(f, "${}:{}", self.name, self.variable_type),
        }
    }
}

impl From<parser::OperationDefinition<'_, String>> for OperationDefinition {
    fn from(value: parser::OperationDefinition<'_, String>) -> Self {
        match value {
            parser::OperationDefinition::Query(query) => OperationDefinition {
                name: query.name,
                operation_kind: Some(OperationKind::Query),
                variable_definitions: match query.variable_definitions.len() {
                    0 => None,
                    _ => Some(
                        query
                            .variable_definitions
                            .into_iter()
                            .map(|v| v.into())
                            .collect(),
                    ),
                },
                selection_set: query.selection_set.into(),
            },
            parser::OperationDefinition::SelectionSet(s) => OperationDefinition {
                name: None,
                operation_kind: Some(OperationKind::Query),
                variable_definitions: None,
                selection_set: s.into(),
            },
            parser::OperationDefinition::Mutation(mutation) => OperationDefinition {
                name: mutation.name,
                operation_kind: Some(OperationKind::Mutation),
                variable_definitions: match mutation.variable_definitions.len() {
                    0 => None,
                    _ => Some(
                        mutation
                            .variable_definitions
                            .into_iter()
                            .map(|v| v.into())
                            .collect(),
                    ),
                },
                selection_set: mutation.selection_set.into(),
            },
            parser::OperationDefinition::Subscription(subscription) => OperationDefinition {
                name: subscription.name,
                operation_kind: Some(OperationKind::Subscription),
                variable_definitions: match subscription.variable_definitions.len() {
                    0 => None,
                    _ => Some(
                        subscription
                            .variable_definitions
                            .into_iter()
                            .map(|v| v.into())
                            .collect(),
                    ),
                },
                selection_set: subscription.selection_set.into(),
            },
        }
    }
}

impl From<&parser::VariableDefinition<'_, String>> for VariableDefinition {
    fn from(value: &parser::VariableDefinition<'_, String>) -> Self {
        VariableDefinition {
            name: value.name.clone(),
            variable_type: (&value.var_type).into(),
            default_value: value.default_value.as_ref().map(|v| v.into()),
        }
    }
}

impl From<parser::VariableDefinition<'_, String>> for VariableDefinition {
    fn from(value: parser::VariableDefinition<'_, String>) -> Self {
        VariableDefinition {
            name: value.name,
            variable_type: (&value.var_type).into(),
            default_value: value.default_value.as_ref().map(|v| v.into()),
        }
    }
}
