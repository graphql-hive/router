use graphql_parser::query::{Definition, OperationDefinition};

use crate::ast::normalization::{context::NormalizationContext, error::NormalizationError};

pub fn drop_unused_operations(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    let mut already_found = false;

    // GraphQL validation stage should prevent:
    // - mix of anonymous and named operations
    // - two operations of the same name
    // - multiple anonymous operations

    ctx.document.definitions.retain_mut(|def| {
        match def {
            Definition::Operation(op) => {
                if already_found {
                    return false;
                }

                if ctx.operation_name.is_none() {
                    already_found = true;
                    return true;
                }

                let bingo = match op {
                    OperationDefinition::Query(q) => q.name.as_deref() == ctx.operation_name,
                    OperationDefinition::Mutation(m) => m.name.as_deref() == ctx.operation_name,
                    OperationDefinition::Subscription(s) => s.name.as_deref() == ctx.operation_name,
                    // If we're looking for named, anonymous should be dropped,
                    // If we're looking for anonymous, first operation will be used.
                    // That's why we drop anonymous here.
                    OperationDefinition::SelectionSet(_) => false,
                };

                if bingo {
                    already_found = true;
                }

                bingo
            }
            Definition::Fragment(_) => true, // Always keep fragment definitions at this pipeline stage
        }
    });

    if !already_found {
        println!("error!");
        if let Some(name) = ctx.operation_name {
            return Err(NormalizationError::SpecifiedOperationNotFound {
                operation_name: name.to_string(),
            });
        } else {
            return Err(NormalizationError::OperationNotFound);
        }
    }

    Ok(())
}
