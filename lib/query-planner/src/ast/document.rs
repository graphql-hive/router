use std::fmt::Display;

use serde::{Deserialize, Serialize};

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
