use super::ValidationRule;
use crate::ast::{
    visit_document, FieldByNameExtension, InputValueHelpers, OperationVisitor,
    OperationVisitorContext,
};
use crate::static_graphql::query::Value;
use crate::static_graphql::schema::InputValue;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Provided required arguments
///
/// A field or directive is only valid if all required (non-null without a
/// default value) field arguments have been provided.
///
/// See https://spec.graphql.org/draft/#sec-Required-Arguments
pub struct ProvidedRequiredArguments;

impl Default for ProvidedRequiredArguments {
    fn default() -> Self {
        Self::new()
    }
}

impl ProvidedRequiredArguments {
    pub fn new() -> Self {
        ProvidedRequiredArguments
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for ProvidedRequiredArguments {
    fn enter_field(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        field: &crate::static_graphql::query::Field,
    ) {
        if let Some(parent_type) = visitor_context.current_parent_type() {
            if let Some(field_def) = parent_type.field_by_name(&field.name) {
                let missing_required_args =
                    validate_arguments(&field.arguments, &field_def.arguments);

                for missing in missing_required_args {
                    user_context.report_error(ValidationError {error_code: self.error_code(),
              locations: vec![field.position],
              message: format!("Field \"{}\" argument \"{}\" of type \"{}\" is required, but it was not provided.",
              field.name, missing.name, missing.value_type),
          });
                }
            }
        }
    }

    fn enter_directive(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        directive: &crate::static_graphql::query::Directive,
    ) {
        let known_directives = &visitor_context.directives;

        if let Some(directive_def) = known_directives.get(&directive.name) {
            let missing_required_args =
                validate_arguments(&directive.arguments, &directive_def.arguments);

            for missing in missing_required_args {
                user_context.report_error(ValidationError {error_code: self.error_code(),
              locations: vec![directive.position],
              message: format!("Directive \"@{}\" argument \"{}\" of type \"{}\" is required, but it was not provided.",
              directive.name, missing.name, missing.value_type),
          });
            }
        }
    }
}

fn validate_arguments(
    arguments_used: &[(String, Value)],
    arguments_defined: &[InputValue],
) -> Vec<InputValue> {
    arguments_defined
        .iter()
        .filter_map(|field_arg_def| {
            if field_arg_def.is_required()
                && !arguments_used
                    .iter()
                    .any(|(name, _value)| name.eq(&field_arg_def.name))
            {
                Some(field_arg_def.clone())
            } else {
                None
            }
        })
        .collect()
}

impl ValidationRule for ProvidedRequiredArguments {
    fn error_code<'a>(&self) -> &'a str {
        "ProvidedRequiredArguments"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut ProvidedRequiredArguments::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn ignores_unknown_arguments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          dog {
            isHouseTrained(unknownArgument: true)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn arg_on_optional_arg() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          dog {
            isHouseTrained(atOtherHomes: true)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn no_arg_on_optional_arg() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          dog {
            isHouseTrained
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn multiple_args() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleReqs(req1: 1, req2: 2)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn multiple_args_reverse_order() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleReqs(req2: 2, req1: 1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn no_args_on_multiple_optional() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleOpts
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn one_arg_on_multiple_optional() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleOpts(opt1: 1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn second_arg_on_multiple_optional() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleOpts(opt2: 1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn multiple_required_args_on_mixed_list() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleOptAndReq(req1: 3, req2: 4)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn multiple_required_and_one_optional_arg_on_mixedlist() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleOptAndReq(req1: 3, req2: 4, opt1: 5)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn all_required_and_optional_args_on_mixedlist() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleOptAndReq(req1: 3, req2: 4, opt1: 5, opt2: 6)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn missing_one_non_nullable_argument() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleReqs(req2: 2)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Field \"multipleReqs\" argument \"req1\" of type \"Int!\" is required, but it was not provided."
    ]);
}

#[test]
fn missing_multiple_non_nullable_arguments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleReqs
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages, vec![
      "Field \"multipleReqs\" argument \"req1\" of type \"Int!\" is required, but it was not provided.",
      "Field \"multipleReqs\" argument \"req2\" of type \"Int!\" is required, but it was not provided."
    ]);
}

#[test]
fn incorrect_value_and_missing_argument() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          complicatedArgs {
            multipleReqs(req1: \"one\")
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Field \"multipleReqs\" argument \"req2\" of type \"Int!\" is required, but it was not provided."
    ]);
}

#[test]
fn ignores_unknown_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          dog @unknown
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn with_directives_of_valid_types() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          dog @include(if: true) {
            name
          }
          human @skip(if: false) {
            name
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn with_directive_with_missing_types() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ProvidedRequiredArguments {}));
    let errors = test_operation_with_schema(
        "{
          dog @include {
            name @skip
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages, vec![
      "Directive \"@include\" argument \"if\" of type \"Boolean!\" is required, but it was not provided.",
      "Directive \"@skip\" argument \"if\" of type \"Boolean!\" is required, but it was not provided."
    ])
}
