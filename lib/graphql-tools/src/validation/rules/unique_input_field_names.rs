use std::collections::HashSet;

use super::ValidationRule;
use crate::ast::{OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::Value;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Unique input field names
///
/// A GraphQL input object is only valid if all supplied fields are
/// uniquely named.
///
/// See https://spec.graphql.org/draft/#sec-Input-Object-Field-Uniqueness
pub struct UniqueInputFieldNames;

impl Default for UniqueInputFieldNames {
    fn default() -> Self {
        Self::new()
    }
}

impl UniqueInputFieldNames {
    pub fn new() -> Self {
        UniqueInputFieldNames
    }
}

impl<'doc> OperationVisitor<'doc, ValidationErrorContext> for UniqueInputFieldNames {
    fn enter_object_value(
        &mut self,
        _: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        object_value: &[(String, Value)],
    ) {
        let mut seen = HashSet::new();
        for (field_name, _) in object_value {
            if !seen.insert(field_name) {
                user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    message: format!(
                        "There can be only one input field named \"{}\".",
                        field_name
                    ),
                    locations: vec![],
                });
            }
        }
    }
}

impl ValidationRule for UniqueInputFieldNames {
    fn error_code(&self) -> &'static str {
        "UniqueInputFieldNames"
    }

    fn visitor<'doc>(&self) -> super::ValidationVisitor<'doc> {
        Box::new(UniqueInputFieldNames::new())
    }
}

#[test]
fn input_object_with_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueInputFieldNames {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            complexArgField(complexArg: { requiredField: true })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn input_object_with_two_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueInputFieldNames {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            complexArgField(complexArg: { requiredField: true, intField: 5 })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn same_input_object_within_two_args() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueInputFieldNames {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            a: complexArgField(complexArg: { requiredField: true })
            b: complexArgField(complexArg: { requiredField: true })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn multiple_input_object_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueInputFieldNames {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            complexArgField(complexArg: { requiredField: true, intField: 5, stringField: \"hello\" })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn allows_for_nested_input_objects_with_similar_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueInputFieldNames {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            complexArgField(complexArg: { requiredField: true, nested: { requiredField: false } })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn duplicate_input_object_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueInputFieldNames {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            complexArgField(complexArg: { requiredField: true, requiredField: true })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["There can be only one input field named \"requiredField\"."]
    );
}
#[test]
fn many_duplicate_input_object_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueInputFieldNames {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            complexArgField(complexArg: { intField: 5, intField: 6, intField: 7 })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec!["There can be only one input field named \"intField\"."; 2]
    );
}

#[test]
fn nested_duplicate_input_object_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueInputFieldNames {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            complexArgField(complexArg: { stringField: \"a\", nested: { intField: 1, intField: 2 } })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["There can be only one input field named \"intField\"."]
    );
}
