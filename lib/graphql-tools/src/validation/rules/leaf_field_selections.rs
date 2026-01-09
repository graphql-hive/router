use super::ValidationRule;
use crate::{
    ast::{visit_document, OperationVisitor, OperationVisitorContext, TypeDefinitionExtension},
    validation::utils::{ValidationError, ValidationErrorContext},
};

/// Leaf Field Selections
///
/// Field selections on scalars or enums are never allowed, because they are the leaf nodes of any GraphQL operation.
///
/// https://spec.graphql.org/draft/#sec-Leaf-Field-Selections
pub struct LeafFieldSelections;

impl Default for LeafFieldSelections {
    fn default() -> Self {
        Self::new()
    }
}

impl LeafFieldSelections {
    pub fn new() -> Self {
        LeafFieldSelections
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for LeafFieldSelections {
    fn enter_field(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        field: &crate::static_graphql::query::Field,
    ) {
        if let (Some(field_type), Some(field_type_literal)) = (
            (visitor_context.current_type()),
            (visitor_context.current_type_literal()),
        ) {
            let field_selection_count = field.selection_set.items.len();

            if field_type.is_leaf_type() {
                if field_selection_count > 0 {
                    user_context.report_error(ValidationError {
                        error_code: self.error_code(),
                        locations: vec![field.position],
                        message: format!(
                  "Field \"{}\" must not have a selection since type \"{}\" has no subfields.",
                  field.name,
                  field_type_literal
              ),
                    });
                }
            } else if field_selection_count == 0 {
                      user_context.report_error(ValidationError {error_code: self.error_code(),
                locations: vec![field.position],
                message: format!(
                    "Field \"{}\" of type \"{}\" must have a selection of subfields. Did you mean \"{} {{ ... }}\"?",
                    field.name,
                    field_type_literal,
                    field.name
                ),
            });
                  }
        }
    }
}

impl ValidationRule for LeafFieldSelections {
    fn error_code<'a>(&self) -> &'a str {
        "LeafFieldSelections"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut LeafFieldSelections::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn valid_scalar_selection() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LeafFieldSelections {}));
    let errors = test_operation_with_schema(
        "fragment scalarSelection on Dog {
          barks
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn object_type_missing_selection() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LeafFieldSelections {}));
    let errors = test_operation_with_schema(
        "query directQueryOnObjectWithoutSubFields {
          human
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Field \"human\" of type \"Human\" must have a selection of subfields. Did you mean \"human { ... }\"?"]
    );
}

#[test]
fn interface_type_missing_selection() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LeafFieldSelections {}));
    let errors = test_operation_with_schema(
        "{
          human { pets }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Field \"pets\" of type \"[Pet]\" must have a selection of subfields. Did you mean \"pets { ... }\"?"]
    );
}

#[test]
fn selection_not_allowed_on_scalar() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LeafFieldSelections {}));
    let errors = test_operation_with_schema(
        "fragment scalarSelectionsNotAllowedOnBoolean on Dog {
          barks { sinceWhen }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Field \"barks\" must not have a selection since type \"Boolean\" has no subfields."]
    );
}

#[test]
fn selection_not_allowed_on_enum() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LeafFieldSelections {}));
    let errors = test_operation_with_schema(
        "fragment scalarSelectionsNotAllowedOnEnum on Cat {
          furColor { inHexDec }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Field \"furColor\" must not have a selection since type \"FurColor\" has no subfields."]
    );
}

#[test]
fn scalar_selection_not_allowed_with_args() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LeafFieldSelections {}));
    let errors = test_operation_with_schema(
        "fragment scalarSelectionsNotAllowedWithArgs on Dog {
          doesKnowCommand(dogCommand: SIT) { sinceWhen }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Field \"doesKnowCommand\" must not have a selection since type \"Boolean\" has no subfields."]
    );
}

#[test]
fn scalar_selection_not_allowed_with_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LeafFieldSelections {}));
    let errors = test_operation_with_schema(
        "fragment scalarSelectionsNotAllowedWithDirectives on Dog {
          name @include(if: true) { isAlsoHumanName }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Field \"name\" must not have a selection since type \"String\" has no subfields."]
    );
}

#[test]
fn scalar_selection_not_allowed_with_directives_and_args() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LeafFieldSelections {}));
    let errors = test_operation_with_schema(
        "fragment scalarSelectionsNotAllowedWithDirectivesAndArgs on Dog {
          doesKnowCommand(dogCommand: SIT) @include(if: true) { sinceWhen }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Field \"doesKnowCommand\" must not have a selection since type \"Boolean\" has no subfields."]
    );
}
