use std::fmt::Display;

use crate::{
    ast::{document::Document, hash::ast_hash},
    state::supergraph_state::TypeNode,
};
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
    pub document: Document,
    pub document_str: String,
}

impl SubgraphFetchOperation {
    pub fn get_inner_selection_set(&self) -> &SelectionSet {
        if let SelectionItem::Field(field) = &self.document.operation.selection_set.items[0] {
            if field.name == "_entities" {
                return &field.selections;
            } else {
                return &self.document.operation.selection_set;
            }
        }

        &self.document.operation.selection_set
    }
}

impl Serialize for SubgraphFetchOperation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.document.to_string())
    }
}

impl PrettyDisplay for SubgraphFetchOperation {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);
        // TODO: improve
        let has_variables = self
            .document
            .operation
            .variable_definitions
            .as_ref()
            .is_some_and(|defs| {
                !defs.is_empty() && defs.iter().all(|v| v.variable_type.inner_type() != "_Any")
            });
        let kind: &str = match &self.document.operation.operation_kind {
            Some(kind) => match kind {
                OperationKind::Query => match has_variables {
                    true => "query ",
                    false => "",
                },
                OperationKind::Mutation => "mutation ",
                OperationKind::Subscription => "subscription ",
            },
            None => "",
        };
        let variables =
            if let Some(variables) = self.document.operation.variable_definitions.as_ref() {
                let representationless = variables
                    .iter()
                    .filter(|v| v.variable_type.inner_type() != "_Any")
                    .collect::<Vec<_>>();

                if representationless.is_empty() {
                    "".to_string()
                } else {
                    format!(
                        "({}) ",
                        representationless
                            .iter()
                            .map(|v| v.to_string())
                            .collect::<Vec<String>>()
                            .join(",")
                    )
                }
            } else {
                "".to_string()
            };
        writeln!(f, "{indent}  {kind}{variables}{{")?;
        self.get_inner_selection_set().pretty_fmt(f, depth + 2)?;
        writeln!(f, "{indent}  }}")?;

        if !self.document.fragments.is_empty() {
            for fragment in &self.document.fragments {
                fragment.pretty_fmt(f, depth)?;
            }
        }

        Ok(())
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
                let len = variable_definitions.len();
                for (i, variable_definition) in variable_definitions.iter().enumerate() {
                    let is_last = i == len - 1;
                    write!(f, "{}", variable_definition)?;

                    if !is_last {
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

impl<'a, T: parser::Text<'a>> From<parser::OperationDefinition<'a, T>> for OperationDefinition {
    fn from(value: parser::OperationDefinition<'a, T>) -> Self {
        match value {
            parser::OperationDefinition::Query(query) => OperationDefinition {
                name: query.name.map(|n| n.as_ref().to_string()),
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
                name: mutation.name.map(|n| n.as_ref().to_string()),
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
                name: subscription.name.map(|n| n.as_ref().to_string()),
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

impl<'a, T: parser::Text<'a>> From<&parser::VariableDefinition<'a, T>> for VariableDefinition {
    fn from(value: &parser::VariableDefinition<'a, T>) -> Self {
        VariableDefinition {
            name: value.name.as_ref().to_string(),
            variable_type: (&value.var_type).into(),
            default_value: value.default_value.as_ref().map(|v| v.into()),
        }
    }
}

impl<'a, T: parser::Text<'a>> From<parser::VariableDefinition<'a, T>> for VariableDefinition {
    fn from(value: parser::VariableDefinition<'a, T>) -> Self {
        VariableDefinition {
            name: value.name.as_ref().to_string(),
            variable_type: (&value.var_type).into(),
            default_value: value.default_value.as_ref().map(|v| v.into()),
        }
    }
}
