use std::fmt::Display;

use graphql_tools::parser::query::{self as parser, ParseError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::query_planner::{
    ast::fragment::FragmentDefinition, utils::parsing::safe_parse_operation,
};

use super::operation::OperationDefinition;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedDocument {
    pub operation: OperationDefinition,
    pub operation_name: Option<String>,
}

impl NormalizedDocument {
    pub fn executable_operation(&self) -> &OperationDefinition {
        &self.operation
    }
}

impl Display for NormalizedDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.operation)?;

        Ok(())
    }
}

pub enum Definition {
    Operation(OperationDefinition),
    Fragment(FragmentDefinition),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub operation: OperationDefinition,
    pub fragments: Vec<FragmentDefinition>,
}

#[derive(Debug, Error)]
pub enum DocumentParseError {
    #[error("failed to parse the operation: {0}")]
    Parse(#[from] ParseError),
    #[error("the operation text contains no operation definition")]
    NoOperationDefinition,
}

impl Document {
    /// Parses operation text back into a document, for cases where only the text was kept.
    pub fn parse_executable(text: &str) -> Result<Self, DocumentParseError> {
        let parsed = safe_parse_operation(text)?;
        let mut operation = None;
        let mut fragments = Vec::new();

        for definition in parsed.definitions {
            match definition {
                parser::Definition::Operation(current) => {
                    if operation.is_none() {
                        operation = Some(current.into());
                    }
                }
                parser::Definition::Fragment(fragment) => fragments.push(fragment.into()),
            }
        }

        Ok(Document {
            operation: operation.ok_or(DocumentParseError::NoOperationDefinition)?,
            fragments,
        })
    }
}

impl Display for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.operation)?;

        if self.fragments.is_empty() {
            Ok(())
        } else {
            writeln!(f, "\n")?;
            for fragment in &self.fragments {
                writeln!(f, "{}", fragment)?;
            }
            Ok(())
        }
    }
}
