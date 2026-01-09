use super::ValidationRule;
use crate::ast::{
    visit_document, OperationVisitor, OperationVisitorContext, SchemaDocumentExtension,
    TypeExtension,
};
use crate::static_graphql::query::TypeCondition;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Known type names
///
/// A GraphQL document is only valid if referenced types (specifically
/// variable definitions and fragment conditions) are defined by the type schema.
///
/// See https://spec.graphql.org/draft/#sec-Fragment-Spread-Type-Existence
pub struct KnownTypeNames;

impl Default for KnownTypeNames {
    fn default() -> Self {
        Self::new()
    }
}

impl KnownTypeNames {
    pub fn new() -> Self {
        KnownTypeNames
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for KnownTypeNames {
    fn enter_fragment_definition(
        &mut self,
        visitor_context: &mut crate::ast::OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        fragment_definition: &crate::static_graphql::query::FragmentDefinition,
    ) {
        let TypeCondition::On(fragment_type_name) = &fragment_definition.type_condition;

        if visitor_context.schema.type_by_name(fragment_type_name).is_none() && !fragment_type_name.starts_with("__") {
            user_context.report_error(ValidationError {
                error_code: self.error_code(),
                locations: vec![fragment_definition.position],
                message: format!("Unknown type \"{}\".", fragment_type_name),
            });
        }
    }

    fn enter_inline_fragment(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        inline_fragment: &crate::static_graphql::query::InlineFragment,
    ) {
        if let Some(TypeCondition::On(fragment_type_name)) = &inline_fragment.type_condition {
            if visitor_context.schema.type_by_name(fragment_type_name).is_none() && !fragment_type_name.starts_with("__") {
                user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    locations: vec![inline_fragment.position],
                    message: format!("Unknown type \"{}\".", fragment_type_name),
                });
            }
        }
    }

    fn enter_variable_definition(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        variable_definition: &crate::static_graphql::query::VariableDefinition,
    ) {
        let base_type = variable_definition.var_type.inner_type();

        if visitor_context.schema.type_by_name(base_type).is_none() && !base_type.starts_with("__") {
            user_context.report_error(ValidationError {
                error_code: self.error_code(),
                locations: vec![variable_definition.position],
                message: format!("Unknown type \"{}\".", base_type),
            });
        }
    }
}

impl ValidationRule for KnownTypeNames {
    fn error_code<'a>(&self) -> &'a str {
        "KnownTypeNames"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut KnownTypeNames::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn known_type_names_are_valid() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownTypeNames {}));
    let errors = test_operation_with_schema(
        "
        query Foo(
          $var: String
          $required: [Int!]!
          $introspectionType: __EnumValue
        ) {
          user(id: 4) {
            pets { ... on Pet { name }, ...PetFields, ... { name } }
          }
        }
        fragment PetFields on Pet {
          name
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn unknown_type_names_are_invalid() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownTypeNames {}));
    let errors = test_operation_with_schema(
        "query Foo($var: [JumbledUpLetters!]!) {
          user(id: 4) {
            name
            pets { ... on Badger { name }, ...PetFields }
          }
        }

        fragment PetFields on Peat {
          name
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages,
        vec![
            "Unknown type \"JumbledUpLetters\".",
            "Unknown type \"Badger\".",
            "Unknown type \"Peat\"."
        ]
    );
}
