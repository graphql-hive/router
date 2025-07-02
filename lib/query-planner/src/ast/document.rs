use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::ast::fragment::FragmentDefinition;

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
