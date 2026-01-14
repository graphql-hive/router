use std::collections::BTreeMap;

use crate::parser::schema::TypeDefinition;

use crate::ast::{
    InputValueHelpers, SchemaDocumentExtension, TypeDefinitionExtension, TypeExtension,
};
use crate::static_graphql::query::Value;
use crate::validation::utils::ValidationError;
use crate::{
    ast::{visit_document, OperationVisitor, OperationVisitorContext},
    validation::utils::ValidationErrorContext,
};

use super::ValidationRule;

pub struct ValuesOfCorrectType {}

impl Default for ValuesOfCorrectType {
    fn default() -> Self {
        Self::new()
    }
}

impl ValuesOfCorrectType {
    pub fn new() -> Self {
        Self {}
    }

    pub fn is_custom_scalar(&self, type_name: &str) -> bool {
        !matches!(type_name, "String" | "Int" | "Float" | "Boolean" | "ID")
    }

    pub fn validate_value(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        raw_value: &Value,
    ) {
        if let Some(input_type) = visitor_context.current_input_type_literal() {
            let named_type = input_type.inner_type();

            if let Some(type_def) = visitor_context.schema.type_by_name(named_type) {
                if !type_def.is_leaf_type() {
                    user_context.report_error(ValidationError {
                        error_code: self.error_code(),
                        message: format!(
                            "Expected value of type \"{}\", found {}.",
                            named_type, raw_value
                        ),
                        locations: vec![],
                    })
                }

                if let TypeDefinition::Scalar(scalar_type_def) = &type_def {
                    match (scalar_type_def.name.as_ref(), raw_value) {
                        ("Int", Value::Int(_))
                        | ("ID", Value::Int(_))
                        | ("ID", Value::String(_))
                        | ("Float", Value::Int(_))
                        | ("Float", Value::Float(_))
                        | ("Boolean", Value::Boolean(_))
                        | ("String", Value::String(_)) => return,
                        (expected, value) => {
                            if self.is_custom_scalar(expected) {
                                return;
                            }

                            user_context.report_error(ValidationError {
                                error_code: self.error_code(),
                                message: format!(
                                    "Expected value of type \"{}\", found {}.",
                                    expected, value
                                ),
                                locations: vec![],
                            })
                        }
                    }
                }

                if let TypeDefinition::Enum(enum_type_def) = &type_def {
                    match raw_value {
                        Value::Enum(enum_value) => {
                            if !enum_type_def.values.iter().any(|v| v.name.eq(enum_value)) {
                                user_context.report_error(ValidationError {
                                    error_code: self.error_code(),
                                    message: format!(
                                        "Value \"{}\" does not exist in \"{}\" enum.",
                                        enum_value, enum_type_def.name
                                    ),
                                    locations: vec![],
                                })
                            }
                        }
                        value => user_context.report_error(ValidationError {
                            error_code: self.error_code(),
                            message: format!(
                                "Enum \"{}\" cannot represent non-enum value: {}",
                                enum_type_def.name, value
                            ),
                            locations: vec![],
                        }),
                    }
                }
            }
        }
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for ValuesOfCorrectType {
    fn enter_null_value(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        _: (),
    ) {
        if let Some(input_type) = visitor_context.current_input_type_literal() {
            if input_type.is_non_null() {
                user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    message: format!("Expected value of type \"{}\", found null", input_type),
                    locations: vec![],
                })
            }
        }
    }

    fn enter_object_value(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        object_value: &BTreeMap<String, Value>,
    ) {
        if let Some(TypeDefinition::InputObject(input_object_def)) =
            visitor_context.current_input_type()
        {
            input_object_def.fields.iter().for_each(|field| {
                if field.is_required() && !object_value.contains_key(&field.name) {
                    user_context.report_error(ValidationError {
                        error_code: self.error_code(),
                        message: format!(
                            "Field \"{}.{}\" of required type \"{}\" was not provided.",
                            input_object_def.name, field.name, field.value_type
                        ),
                        locations: vec![],
                    })
                }
            });

            object_value.keys().for_each(|field_name| {
                if !input_object_def
                    .fields
                    .iter()
                    .any(|f| f.name.eq(field_name))
                {
                    user_context.report_error(ValidationError {
                        error_code: self.error_code(),
                        message: format!(
                            "Field \"{}\" is not defined by type \"{}\".",
                            field_name, input_object_def.name
                        ),
                        locations: vec![],
                    })
                }
            });
        }
    }

    fn enter_enum_value(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        value: &String,
    ) {
        self.validate_value(visitor_context, user_context, &Value::Enum(value.clone()));
    }

    fn enter_scalar_value(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        value: &Value,
    ) {
        self.validate_value(visitor_context, user_context, value);
    }
}

impl ValidationRule for ValuesOfCorrectType {
    fn error_code<'a>(&self) -> &'a str {
        "ValuesOfCorrectType"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut ValuesOfCorrectType::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn valid_int_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            intArgField(intArg: 2)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_negative_int_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            intArgField(intArg: -2)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_boolean_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            booleanArgField(booleanArg: true)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_string_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringArgField(stringArg: \"foo\")
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_float_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            floatArgField(floatArg: 1.1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_negative_float_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            floatArgField(floatArg: -1.1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_int_into_float_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            floatArgField(floatArg: 1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_int_into_id_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            idArgField(idArg: 1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_string_into_id_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            idArgField(idArg: \"someIdString\")
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_enum_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          dog {
            doesKnowCommand(dogCommand: SIT)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn enum_undefined_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            enumArgField(enumArg: UNKNOWN)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn enum_null_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            enumArgField(enumArg: NO_FUR)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_null_into_nullable() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            intArgField(intArg: null)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);

    let errors = test_operation_with_schema(
        "
        {
          dog(a: null, b: null, c:{ requiredField: true, intField: null }) {
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
fn invalid_int_into_string() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringArgField(stringArg: 1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"String\", found 1."]
    );
}

#[test]
fn invalid_float_into_string() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringArgField(stringArg: 1.0)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"String\", found 1."]
    );
}

#[test]
fn invalid_bool_into_string() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringArgField(stringArg: true)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"String\", found true."]
    );
}

#[test]
fn unquoted_string_to_string() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringArgField(stringArg: BAR)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"String\", found BAR."]
    );
}

#[test]
fn invalid_string_into_int() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            intArgField(intArg: \"3\")
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Int\", found \"3\"."]
    );
}

#[test]
#[ignore = "today this one is handled in graphql_parser so we cant validate it here, but it will panic for sure"]
fn bigint_into_int() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            intArgField(intArg: 829384293849283498239482938)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Int cannot represent non 32-bit signed integer value: 829384293849283498239482938"]
    );
}

#[test]
fn unquoted_string_into_int() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            intArgField(intArg: FOO)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Expected value of type \"Int\", found FOO."]);
}

#[test]
fn simple_float_into_int() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            intArgField(intArg: 3.0)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Expected value of type \"Int\", found 3."]);
}

#[test]
fn float_into_int() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            intArgField(intArg: 3.333)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Int\", found 3.333."]
    );
}

#[test]
fn string_into_float() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            floatArgField(floatArg: \"3.333\")
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Float\", found \"3.333\"."]
    );
}

#[test]
fn boolean_into_float() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            floatArgField(floatArg: true)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Float\", found true."]
    );
}

#[test]
fn unquoted_into_float() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            floatArgField(floatArg: FOO)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Float\", found FOO."]
    );
}

#[test]
fn int_into_boolean() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            booleanArgField(booleanArg: 2)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Boolean\", found 2."]
    );
}

#[test]
fn float_into_boolean() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            booleanArgField(booleanArg: 2.0)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Boolean\", found 2."]
    );
}

#[test]
fn string_into_boolean() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            booleanArgField(booleanArg: \"true\")
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Boolean\", found \"true\"."]
    );
}

#[test]
fn unquoted_into_boolean() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            booleanArgField(booleanArg: TRUE)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Boolean\", found TRUE."]
    );
}

#[test]
fn float_into_id() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            idArgField(idArg: 1.0)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Expected value of type \"ID\", found 1."]);
}

#[test]
fn bool_into_id() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            idArgField(idArg: true)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Expected value of type \"ID\", found true."]);
}

#[test]
fn unquoted_into_id() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            idArgField(idArg: SOMETHING)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"ID\", found SOMETHING."]
    );
}

#[test]
fn int_into_enum() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          dog {
            doesKnowCommand(dogCommand: 2)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Enum \"DogCommand\" cannot represent non-enum value: 2"]
    );
}

#[test]
fn float_into_enum() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          dog {
            doesKnowCommand(dogCommand: 1.0)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Enum \"DogCommand\" cannot represent non-enum value: 1"]
    );
}

#[test]
fn string_into_enum() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          dog {
            doesKnowCommand(dogCommand: \"SIT\")
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Enum \"DogCommand\" cannot represent non-enum value: \"SIT\""]
    );
}

#[test]
fn boolean_into_enum() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          dog {
            doesKnowCommand(dogCommand: true)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Enum \"DogCommand\" cannot represent non-enum value: true"]
    );
}

#[test]
fn unknown_enum_value_into_enum() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          dog {
            doesKnowCommand(dogCommand: JUGGLE)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Value \"JUGGLE\" does not exist in \"DogCommand\" enum."]
    );
}

#[test]
fn different_case_enum_value_into_enum() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          dog {
            doesKnowCommand(dogCommand: sit)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Value \"sit\" does not exist in \"DogCommand\" enum."]
    );
}

#[test]
fn valid_list_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringListArgField(stringListArg: [\"one\", null, \"two\"])
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_empty_list_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringListArgField(stringListArg: [])
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_null_list_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringListArgField(stringListArg: null)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_single_value_into_list_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringListArgField(stringListArg: \"one\")
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn incorrect_item_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringListArgField(stringListArg: [\"one\", 2])
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"String\", found 2."]
    );
}

#[test]
fn single_value_of_incorrect_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            stringListArgField(stringListArg: 1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"String\", found 1."]
    );
}

#[test]
fn arg_on_optional_arg() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          dog {
            isHouseTrained(atOtherHomes: true)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn no_arg_on_optional_arg() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          dog {
            isHouseTrained
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn multiple_valid_args() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleReqs(req1: 1, req2: 2)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn multiple_valid_args_reverse_oreder() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleReqs(req2: 2, req1: 1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn no_args_multiple_optional() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleOpts
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn one_arg_multiple_optinals() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleOpts(opt1: 1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn second_arg_multiple_optinals() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleOpts(opt2: 1)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn multiple_required_args_on_mixed_list() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleOptAndReq(req1: 3, req2: 4)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn multiple_required_args_and_one_optional_on_mixed_list() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleOptAndReq(req1: 3, req2: 4, opt1: 5)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn all_required_and_one_optional() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleOptAndReq(req1: 3, req2: 4, opt1: 5, opt2: 6)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn incorrect_value_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleReqs(req2: \"two\", req1: \"one\")
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
            "Expected value of type \"Int\", found \"two\".",
            "Expected value of type \"Int\", found \"one\"."
        ]
    )
}

#[test]
fn incorrect_value_and_missing_argument() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleReqs(req1: \"one\")
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Int\", found \"one\"."]
    );
}

#[test]
fn null_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            multipleReqs(req1: null)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Int!\", found null"]
    );
}

#[test]
fn optional_arg_despite_required_field_in_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            complexArgField
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn partial_object_only_required() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            complexArgField(complexArg: { requiredField: true })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn partial_object_required_field_can_be_falsy() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            complexArgField(complexArg: { requiredField: false })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn partial_object_including_required() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            complexArgField(complexArg: { requiredField: true, intField: 4 })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn full_object() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            complexArgField(complexArg: {
              requiredField: true,
              intField: 4,
              stringField: \"foo\",
              booleanField: false,
              stringListField: [\"one\", \"two\"]
            })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn full_object_with_fields_in_different_order() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            complexArgField(complexArg: {
              stringListField: [\"one\", \"two\"],
              booleanField: false,
              requiredField: true,
              stringField: \"foo\",
              intField: 4,
            })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn partial_object_missing_required() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            complexArgField(complexArg: { intField: 4 })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
            "Field \"ComplexInput.requiredField\" of required type \"Boolean!\" was not provided."
        ]
    )
}

#[test]
fn partial_object_invalid_field_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            complexArgField(complexArg: {
              stringListField: [\"one\", 2],
              requiredField: true,
            })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"String\", found 2."]
    )
}

#[test]
fn partial_object_null_to_non_null_field() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            complexArgField(complexArg: {
              requiredField: true,
              nonNullField: null,
            })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"Boolean!\", found null"]
    )
}

#[test]
fn partial_object_unknown_field_arg() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          complicatedArgs {
            complexArgField(complexArg: {
              requiredField: true,
              invalidField: \"value\"
            })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Field \"invalidField\" is not defined by type \"ComplexInput\"."]
    )
}

#[test]
fn allows_custom_scalar_to_accept_complex_literals() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          test1: anyArg(arg: 123)
          test2: anyArg(arg: \"abc\")
          test3: anyArg(arg: [123, \"abc\"])
          test4: anyArg(arg: {deep: [123, \"abc\"]})
        }",
        "
        scalar Any

        type Query {
          anyArg(arg: Any): String
        }
        ",
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn with_directives_of_valid_types() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
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
fn with_directives_of_invalid_types() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        {
          dog @include(if: \"yes\") {
            name @skip(if: ENUM)
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
            "Expected value of type \"Boolean\", found \"yes\".",
            "Expected value of type \"Boolean\", found ENUM."
        ]
    )
}

#[test]
fn variables_with_valid_default_values() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        query WithDefaultValues(
          $a: Int = 1,
          $b: String = \"ok\",
          $c: ComplexInput = { requiredField: true, intField: 3 }
          $d: Int! = 123
        ) {
          dog { name }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn variables_with_valid_default_null_values() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        query WithDefaultValues(
          $a: Int = null,
          $b: String = null,
          $c: ComplexInput = { requiredField: true, intField: null }
        ) {
          dog { name }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn variables_with_invalid_default_null_values() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        query WithDefaultValues(
          $a: Int! = null,
          $b: String! = null,
          $c: ComplexInput = { requiredField: null, intField: null }
        ) {
          dog { name }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages,
        vec![
            "Expected value of type \"Int!\", found null",
            "Expected value of type \"String!\", found null",
            "Expected value of type \"Boolean!\", found null"
        ]
    );
}

#[test]
fn variables_with_invalid_default_values() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        query InvalidDefaultValues(
          $a: Int = \"one\",
          $b: String = 4,
          $c: ComplexInput = \"NotVeryComplex\"
        ) {
          dog { name }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages,
        vec![
            "Expected value of type \"Int\", found \"one\".",
            "Expected value of type \"String\", found 4.",
            "Expected value of type \"ComplexInput\", found \"NotVeryComplex\"."
        ]
    );
}

#[test]
fn variables_with_complex_invalid_default_values() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        query WithDefaultValues(
          $a: ComplexInput = { requiredField: 123, intField: \"abc\" }
        ) {
          dog { name }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec![
            "Expected value of type \"Int\", found \"abc\".",
            "Expected value of type \"Boolean\", found 123.",
        ]
    );
}

#[test]
fn complex_variables_missing_required_field() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        query MissingRequiredField($a: ComplexInput = {intField: 3}) {
          dog { name }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
            "Field \"ComplexInput.requiredField\" of required type \"Boolean!\" was not provided.",
        ]
    );
}

#[test]
fn list_variables_with_invalid_item() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(ValuesOfCorrectType::new()));
    let errors = test_operation_with_schema(
        "
        query InvalidItem($a: [String] = [\"one\", 2]) {
          dog { name }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Expected value of type \"String\", found 2."]
    );
}
