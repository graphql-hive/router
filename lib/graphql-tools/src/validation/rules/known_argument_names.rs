use super::ValidationRule;
use crate::ast::ext::TypeDefinitionExtension;
use crate::ast::{
    visit_document, FieldByNameExtension, OperationVisitor, OperationVisitorContext,
    SchemaDocumentExtension,
};
use crate::static_graphql::query::Directive;
use crate::static_graphql::schema::{InputValue, TypeDefinition};
use crate::validation::utils::{ValidationError, ValidationErrorContext};
/// Known argument names
///
/// A GraphQL field/directive is only valid if all supplied arguments are defined by
/// that field.
///
/// See https://spec.graphql.org/draft/#sec-Argument-Names
/// See https://spec.graphql.org/draft/#sec-Directives-Are-In-Valid-Locations
pub struct KnownArgumentNames<'a> {
    current_known_arguments: Option<(ArgumentParent<'a>, &'a Vec<InputValue>)>,
}

#[derive(Debug)]
enum ArgumentParent<'a> {
    Field(&'a str, &'a TypeDefinition),
    Directive(&'a str),
}

impl<'a> Default for KnownArgumentNames<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> KnownArgumentNames<'a> {
    pub fn new() -> Self {
        KnownArgumentNames {
            current_known_arguments: None,
        }
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for KnownArgumentNames<'a> {
    fn enter_directive(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        directive: &Directive,
    ) {
        if let Some(directive_def) = visitor_context.schema.directive_by_name(&directive.name) {
            self.current_known_arguments = Some((
                ArgumentParent::Directive(&directive_def.name),
                &directive_def.arguments,
            ));
        }
    }

    fn leave_directive(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        _: &crate::static_graphql::query::Directive,
    ) {
        self.current_known_arguments = None;
    }

    fn enter_field(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        field: &crate::static_graphql::query::Field,
    ) {
        if let Some(parent_type) = visitor_context.current_parent_type() {
            if let Some(field_def) = parent_type.field_by_name(&field.name) {
                self.current_known_arguments = Some((
                    ArgumentParent::Field(
                        &field_def.name,
                        visitor_context
                            .current_parent_type()
                            .expect("Missing parent type"),
                    ),
                    &field_def.arguments,
                ));
            }
        }
    }

    fn leave_field(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        _: &crate::static_graphql::query::Field,
    ) {
        self.current_known_arguments = None;
    }

    fn enter_argument(
        &mut self,
        _: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        (argument_name, _argument_value): &(String, crate::static_graphql::query::Value),
    ) {
        if let Some((arg_position, args)) = &self.current_known_arguments {
            if !args.iter().any(|a| a.name.eq(argument_name)) {
                match arg_position {
                    ArgumentParent::Field(field_name, type_name) => {
                        user_context.report_error(ValidationError {
                            error_code: self.error_code(),
                            message: format!(
                                "Unknown argument \"{}\" on field \"{}.{}\".",
                                argument_name,
                                type_name.name(),
                                field_name
                            ),
                            locations: vec![],
                        })
                    }
                    ArgumentParent::Directive(directive_name) => {
                        user_context.report_error(ValidationError {
                            error_code: self.error_code(),
                            message: format!(
                                "Unknown argument \"{}\" on directive \"@{}\".",
                                argument_name, directive_name
                            ),
                            locations: vec![],
                        })
                    }
                };
            }
        }
    }
}

impl<'k> ValidationRule for KnownArgumentNames<'k> {
    fn error_code<'a>(&self) -> &'a str {
        "KnownArgumentNames"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut KnownArgumentNames::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn single_arg_is_known() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "fragment argOnRequiredArg on Dog {
          doesKnowCommand(dogCommand: SIT)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn multple_args_are_known() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "fragment multipleArgs on ComplicatedArgs {
          multipleReqs(req1: 1, req2: 2)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn ignores_args_of_unknown_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "fragment argOnUnknownField on Dog {
          unknownField(unknownArg: SIT)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn multiple_args_in_reverse_order_are_known() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "fragment multipleArgsReverseOrder on ComplicatedArgs {
          multipleReqs(req2: 2, req1: 1)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn no_args_on_optional_arg() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "fragment noArgOnOptionalArg on Dog {
          isHouseTrained
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn args_are_known_deeply() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "{
          dog {
            doesKnowCommand(dogCommand: SIT)
          }
          human {
            pet {
              ... on Dog {
                doesKnowCommand(dogCommand: SIT)
              }
            }
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn directive_args_are_known() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "{
          dog @skip(if: true)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn field_args_are_invalid() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "{
          dog @skip(unless: true)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Unknown argument \"unless\" on directive \"@skip\"."]
    );
}

#[test]
fn directive_without_args_is_valid() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        " {
          dog @onField
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn arg_passed_to_directive_without_arg_is_reported() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        " {
          dog @onField(if: true)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Unknown argument \"if\" on directive \"@onField\"."]
    );
}

#[test]
#[ignore = "Suggestions are not yet supported"]
fn misspelled_directive_args_are_reported() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "{
          dog @skip(iff: true)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Unknown argument \"iff\" on directive \"@onField\". Did you mean \"if\"?"]
    );
}

#[test]
fn invalid_arg_name() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "fragment invalidArgName on Dog {
          doesKnowCommand(unknown: true)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Unknown argument \"unknown\" on field \"Dog.doesKnowCommand\"."]
    );
}

#[test]
#[ignore = "Suggestions are not yet supported"]
fn misspelled_arg_name_is_reported() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "fragment invalidArgName on Dog {
          doesKnowCommand(DogCommand: true)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Unknown argument \"DogCommand\" on field \"Dog.doesKnowCommand\". Did you mean \"dogCommand\"?"]
    );
}

#[test]
fn unknown_args_amongst_known_args() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "fragment oneGoodArgOneInvalidArg on Dog {
          doesKnowCommand(whoKnows: 1, dogCommand: SIT, unknown: true)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec![
            "Unknown argument \"whoKnows\" on field \"Dog.doesKnowCommand\".",
            "Unknown argument \"unknown\" on field \"Dog.doesKnowCommand\"."
        ]
    );
}

#[test]
fn unknown_args_deeply() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownArgumentNames::new()));
    let errors = test_operation_with_schema(
        "{
          dog {
            doesKnowCommand(unknown: true)
          }
          human {
            pet {
              ... on Dog {
                doesKnowCommand(unknown: true)
              }
            }
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec![
            "Unknown argument \"unknown\" on field \"Dog.doesKnowCommand\".",
            "Unknown argument \"unknown\" on field \"Dog.doesKnowCommand\"."
        ]
    );
}
