use std::fmt::Display;

use super::operation::OperationDefinition;

#[derive(Debug, Clone)]
pub struct NormalizedDocument {
    pub operations: Vec<OperationDefinition>,
    pub operation_name: Option<String>,
}

impl NormalizedDocument {
    pub fn executable_operation(&self) -> Option<&OperationDefinition> {
        match self.operation_name {
            Some(ref name) => {
                if let Some(op) = self
                    .operations
                    .iter()
                    .find(|op| op.name.as_ref().is_some_and(|op_name| op_name == name))
                {
                    return Some(op);
                }

                None
            }
            // TODO: improve this logic, based on the GraphQL spec:
            // "If no operation is supplied [to execute], the document must include exactly one operation."
            // "If the document contains multiple operations, the operation name must be supplied to indicate which operation to execute."
            None => self.operations.first(),
        }
    }
}

impl Display for NormalizedDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for operation in &self.operations {
            writeln!(f, "{}", operation)?;
        }

        Ok(())
    }
}
