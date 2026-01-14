use std::collections::HashMap;

use crate::parser::Pos;

use super::ValidationRule;
use crate::ast::{visit_document, OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::Value;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Unique argument names
///
/// A GraphQL field or directive is only valid if all supplied arguments are
/// uniquely named.
///
/// See https://spec.graphql.org/draft/#sec-Argument-Names
pub struct UniqueArgumentNames;

impl Default for UniqueArgumentNames {
    fn default() -> Self {
        Self::new()
    }
}

impl UniqueArgumentNames {
    pub fn new() -> Self {
        UniqueArgumentNames
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for UniqueArgumentNames {
    fn enter_field(
        &mut self,
        _: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        field: &crate::static_graphql::query::Field,
    ) {
        let found_args = collect_from_arguments(field.position, &field.arguments);

        found_args.iter().for_each(|(arg_name, positions)| {
            if positions.len() > 1 {
                user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    message: format!("There can be only one argument named \"{}\".", arg_name),
                    locations: positions.clone(),
                })
            }
        });
    }

    fn enter_directive(
        &mut self,
        _: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        directive: &crate::static_graphql::query::Directive,
    ) {
        let found_args = collect_from_arguments(directive.position, &directive.arguments);

        found_args.iter().for_each(|(arg_name, positions)| {
            if positions.len() > 1 {
                user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    message: format!("There can be only one argument named \"{}\".", arg_name),
                    locations: positions.clone(),
                })
            }
        });
    }
}

fn collect_from_arguments(
    reported_position: Pos,
    arguments: &Vec<(String, Value)>,
) -> HashMap<String, Vec<Pos>> {
    let mut found_args = HashMap::<String, Vec<Pos>>::new();

    for (arg_name, _arg_value) in arguments {
        found_args
            .entry(arg_name.clone())
            .or_default()
            .push(reported_position);
    }

    found_args
}

impl ValidationRule for UniqueArgumentNames {
    fn error_code<'a>(&self) -> &'a str {
        "UniqueArgumentNames"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut UniqueArgumentNames::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn no_arguments_on_field() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn no_arguments_on_directive() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field @directive
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn argument_on_field() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field(arg: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn argument_on_directive() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field @directive(arg: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn same_argument_on_two_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          one: field(arg: \"value\")
          two: field(arg: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn same_argument_on_field_and_directive() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field(arg: \"value\") @directive(arg: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn same_argument_on_two_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field @directive1(arg: \"value\") @directive2(arg: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn multiple_field_arguments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field(arg1: \"value\", arg2: \"value\", arg3: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn multiple_directive_argument() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field @directive(arg1: \"value\", arg2: \"value\", arg3: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn duplicate_field_arguments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field(arg1: \"value\", arg1: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["There can be only one argument named \"arg1\"."]
    );
}

#[test]
fn many_duplicate_field_arguments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field(arg1: \"value\", arg1: \"value\", arg1: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["There can be only one argument named \"arg1\"."]
    );
}

#[test]
fn duplicate_directive_arguments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field @directive(arg1: \"value\", arg1: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["There can be only one argument named \"arg1\"."]
    );
}

#[test]
fn many_duplicate_directive_arguments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueArgumentNames {}));
    let errors = test_operation_with_schema(
        "{
          field @directive(arg1: \"value\", arg1: \"value\", arg1: \"value\")
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["There can be only one argument named \"arg1\"."]
    );
}
