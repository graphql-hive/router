use super::ValidationRule;
use crate::ast::{
    visit_document, OperationVisitor, OperationVisitorContext, SchemaDocumentExtension,
    TypeDefinitionExtension, TypeExtension,
};
use crate::validation::utils::ValidationError;
use crate::validation::utils::ValidationErrorContext;

/// Variables are input types
///
/// A GraphQL operation is only valid if all the variables it defines are of
/// input types (scalar, enum, or input object).
///
/// See https://spec.graphql.org/draft/#sec-Variables-Are-Input-Types
#[derive(Default)]
pub struct VariablesAreInputTypes;

impl VariablesAreInputTypes {
    pub fn new() -> Self {
        VariablesAreInputTypes
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for VariablesAreInputTypes {
    fn enter_variable_definition(
        &mut self,
        context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        variable_definition: &crate::static_graphql::query::VariableDefinition,
    ) {
        if let Some(var_schema_type) = context
            .schema
            .type_by_name(variable_definition.var_type.inner_type())
        {
            if !var_schema_type.is_input_type() {
                user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    message: format!(
                        "Variable \"${}\" cannot be non-input type \"{}\".",
                        variable_definition.name, variable_definition.var_type
                    ),
                    locations: vec![variable_definition.position],
                })
            }
        }
    }
}

impl ValidationRule for VariablesAreInputTypes {
    fn error_code<'a>(&self) -> &'a str {
        "VariablesAreInputTypes"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut VariablesAreInputTypes::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn unknown_types_are_ignored() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesAreInputTypes {}));
    let errors = test_operation_with_schema(
        "
        query Foo($a: Unknown, $b: [[Unknown!]]!) {
          field(a: $a, b: $b)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn input_types_are_valid() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesAreInputTypes {}));
    let errors = test_operation_with_schema(
        "
        query Foo($a: String, $b: [Boolean!]!, $c: ComplexInput) {
          field(a: $a, b: $b, c: $c)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn output_types_are_invalid() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesAreInputTypes {}));
    let errors = test_operation_with_schema(
        "
       query Foo($a: Dog, $b: [[CatOrDog!]]!, $c: Pet) {
        field(a: $a, b: $b, c: $c)
      }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages,
        vec![
            "Variable \"$a\" cannot be non-input type \"Dog\".",
            "Variable \"$b\" cannot be non-input type \"[[CatOrDog!]]!\".",
            "Variable \"$c\" cannot be non-input type \"Pet\".",
        ]
    );
}
