use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::query_planner::ast::fragment::FragmentDefinition;

use super::operation::OperationDefinition;
use super::shrink::ShrinkMemory;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedDocument {
    pub operation_name: Option<String>,
    /// The same operation, flat, for the planner.
    #[serde(skip)]
    pub plan_operation: std::sync::Arc<super::flat::selection_tree::PlanOperation>,
    /// The normalized operation, flat.
    #[serde(skip)]
    pub document: std::sync::Arc<super::flat::plan_document::PlanDocument>,
}

impl NormalizedDocument {
    /// The operation as a tree, exported from the flat document. For tools and tests, the
    /// request path never builds one.
    pub fn operation_tree(&self) -> OperationDefinition {
        self.document.operation_tree()
    }
}

impl Display for NormalizedDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.document)?;

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

impl ShrinkMemory for Document {
    fn shrink_memory(&mut self) {
        self.operation.shrink_memory();
        self.fragments.shrink_memory();
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
