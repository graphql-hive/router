use graphql_parser::query::{Definition, OperationDefinition};

use crate::ast::normalization::{context::NormalizationContext, error::NormalizationError};

fn equal_names(name: &Option<String>, expected_name: Option<&str>) -> bool {
    name.as_deref() == expected_name
}

pub fn drop_unused_operations(ctx: &mut NormalizationContext) -> Result<(), NormalizationError> {
    let mut found_already = false;
    let mut error_inside_retain: Option<NormalizationError> = None;

    ctx.document.definitions.retain_mut(|def| {
        if error_inside_retain.is_some() {
            return false;
        }

        match def {
            Definition::Operation(op) => {
                let current_op_name_in_ast: Option<&Option<String>>;
                let is_anonymous: bool;

                match op {
                    OperationDefinition::Query(q) => {
                        current_op_name_in_ast = Some(&q.name);
                        is_anonymous = q.name.is_none();
                    }
                    OperationDefinition::Mutation(m) => {
                        current_op_name_in_ast = Some(&m.name);
                        is_anonymous = m.name.is_none();
                    }
                    OperationDefinition::Subscription(s) => {
                        current_op_name_in_ast = Some(&s.name);
                        is_anonymous = s.name.is_none();
                    }
                    OperationDefinition::SelectionSet(_) => {
                        current_op_name_in_ast = None;
                        is_anonymous = true;
                    }
                }

                let is_candidate = if is_anonymous {
                    ctx.operation_name.is_none()
                } else if let Some(name_opt_ref) = current_op_name_in_ast {
                    equal_names(name_opt_ref, ctx.operation_name)
                } else {
                    false
                };

                if is_candidate {
                    if found_already {
                        error_inside_retain =
                            Some(NormalizationError::MultipleMatchingOperationsFound);
                        return false;
                    }
                    found_already = true;
                    true
                } else {
                    false
                }
            }
            Definition::Fragment(_) => true, // Always keep fragment definitions at this pipeline stage
        }
    });

    if let Some(err) = error_inside_retain {
        return Err(err);
    }

    if !found_already {
        if let Some(name) = ctx.operation_name {
            return Err(NormalizationError::SpecifiedOperationNotFound {
                operation_name: name.to_string(),
            });
        } else {
            return Err(NormalizationError::AnonymousOperationNotFound);
        }
    }

    Ok(())
}
