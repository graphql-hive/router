use super::ValidationRule;
use crate::ast::{
    visit_document, OperationVisitor, OperationVisitorContext, SchemaDocumentExtension,
    TypeDefinitionExtension,
};
use crate::static_graphql::query::*;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Fragments on composite type
///
/// Fragments use a type condition to determine if they apply, since fragments
/// can only be spread into a composite type (object, interface, or union), the
/// type condition must also be a composite type.
///
/// https://spec.graphql.org/draft/#sec-Fragments-On-Composite-Types
pub struct FragmentsOnCompositeTypes;

impl Default for FragmentsOnCompositeTypes {
    fn default() -> Self {
        Self::new()
    }
}

impl FragmentsOnCompositeTypes {
    pub fn new() -> Self {
        FragmentsOnCompositeTypes
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for FragmentsOnCompositeTypes {
    fn enter_inline_fragment(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        inline_fragment: &InlineFragment,
    ) {
        if let Some(TypeCondition::On(type_condition)) = &inline_fragment.type_condition {
            if let Some(gql_type) = visitor_context.schema.type_by_name(type_condition) {
                if !gql_type.is_composite_type() {
                    user_context.report_error(ValidationError {
                        locations: vec![inline_fragment.position],
                        error_code: self.error_code(),
                        message: format!(
                            "Fragment cannot condition on non composite type \"{}\".",
                            type_condition
                        ),
                    })
                }
            }
        }
    }

    fn enter_fragment_definition(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        fragment_definition: &FragmentDefinition,
    ) {
        let TypeCondition::On(type_condition) = &fragment_definition.type_condition;

        if let Some(gql_type) = visitor_context.schema.type_by_name(type_condition) {
            if !gql_type.is_composite_type() {
                user_context.report_error(ValidationError {
                    locations: vec![fragment_definition.position],
                    error_code: self.error_code(),
                    message: format!(
                        "Fragment \"{}\" cannot condition on non composite type \"{}\".",
                        fragment_definition.name, type_condition
                    ),
                })
            }
        }
    }
}

impl ValidationRule for FragmentsOnCompositeTypes {
    fn error_code<'a>(&self) -> &'a str {
        "FragmentsOnCompositeTypes"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut FragmentsOnCompositeTypes::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn object_is_valid_fragment_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FragmentsOnCompositeTypes {}));
    let errors = test_operation_with_schema(
        "fragment validFragment on Dog {
          barks
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn interface_is_valid_fragment_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FragmentsOnCompositeTypes {}));
    let errors = test_operation_with_schema(
        "fragment validFragment on Pet {
          name
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn object_is_valid_inline_fragment_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FragmentsOnCompositeTypes {}));
    let errors = test_operation_with_schema(
        "fragment validFragment on Pet {
          ... on Dog {
            barks
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn interface_is_valid_inline_fragment_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FragmentsOnCompositeTypes {}));
    let errors = test_operation_with_schema(
        "fragment validFragment on Mammal {
          ... on Canine {
            name
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn inline_fragment_without_type_is_valid() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FragmentsOnCompositeTypes {}));
    let errors = test_operation_with_schema(
        "fragment validFragment on Pet {
          ... {
            name
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn union_is_valid_fragment_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FragmentsOnCompositeTypes {}));
    let errors = test_operation_with_schema(
        "fragment validFragment on CatOrDog {
          __typename
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn scalar_is_invalid_fragment_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FragmentsOnCompositeTypes {}));
    let errors = test_operation_with_schema(
        "fragment scalarFragment on Boolean {
          bad
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Fragment \"scalarFragment\" cannot condition on non composite type \"Boolean\"."]
    );
}

#[test]
fn enum_is_invalid_fragment_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FragmentsOnCompositeTypes {}));
    let errors = test_operation_with_schema(
        "fragment scalarFragment on FurColor {
          bad
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Fragment \"scalarFragment\" cannot condition on non composite type \"FurColor\"."]
    );
}

#[test]
fn input_object_is_invalid_fragment_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FragmentsOnCompositeTypes {}));
    let errors = test_operation_with_schema(
        "fragment inputFragment on ComplexInput {
          stringField
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Fragment \"inputFragment\" cannot condition on non composite type \"ComplexInput\"."]
    );
}

#[test]
fn scalar_is_invalid_inline_fragment_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FragmentsOnCompositeTypes {}));
    let errors = test_operation_with_schema(
        "fragment invalidFragment on Pet {
          ... on String {
            barks
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Fragment cannot condition on non composite type \"String\"."]
    );
}
