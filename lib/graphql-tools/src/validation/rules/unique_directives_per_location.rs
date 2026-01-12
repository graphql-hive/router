use std::collections::HashSet;

use super::ValidationRule;
use crate::ast::OperationDefinitionExtension;
use crate::static_graphql::query::{
    Directive, Field, FragmentDefinition, FragmentSpread, InlineFragment, OperationDefinition,
};
use crate::{
    ast::{visit_document, OperationVisitor, OperationVisitorContext},
    validation::utils::{ValidationError, ValidationErrorContext},
};

/// Unique directive names per location
///
/// A GraphQL document is only valid if all non-repeatable directives at
/// a given location are uniquely named.
///
/// See  https://spec.graphql.org/draft/#sec-Directives-Are-Unique-Per-Location
pub struct UniqueDirectivesPerLocation {}

impl Default for UniqueDirectivesPerLocation {
    fn default() -> Self {
        Self::new()
    }
}

impl UniqueDirectivesPerLocation {
    pub fn new() -> Self {
        UniqueDirectivesPerLocation {}
    }

    pub fn check_duplicate_directive(
        &self,
        ctx: &mut OperationVisitorContext,
        err_context: &mut ValidationErrorContext,
        directives: &[Directive],
    ) {
        let mut exists = HashSet::new();

        for directive in directives {
            if let Some(meta_directive) = ctx.directives.get(&directive.name) {
                if !meta_directive.repeatable {
                    if exists.contains(&directive.name) {
                        err_context.report_error(ValidationError {
                            error_code: self.error_code(),
                            locations: vec![directive.position],
                            message: format!("Duplicate directive \"{}\"", &directive.name),
                        });

                        continue;
                    }

                    exists.insert(directive.name.clone());
                }
            }
        }
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for UniqueDirectivesPerLocation {
    fn enter_operation_definition(
        &mut self,
        ctx: &mut OperationVisitorContext<'a>,
        err_ctx: &mut ValidationErrorContext,
        operation: &OperationDefinition,
    ) {
        self.check_duplicate_directive(ctx, err_ctx, operation.directives());
    }

    fn enter_field(
        &mut self,
        ctx: &mut OperationVisitorContext<'a>,
        err_ctx: &mut ValidationErrorContext,
        field: &Field,
    ) {
        self.check_duplicate_directive(ctx, err_ctx, &field.directives);
    }

    fn enter_fragment_definition(
        &mut self,
        ctx: &mut OperationVisitorContext<'a>,
        err_ctx: &mut ValidationErrorContext,
        fragment: &FragmentDefinition,
    ) {
        self.check_duplicate_directive(ctx, err_ctx, &fragment.directives);
    }

    fn enter_fragment_spread(
        &mut self,
        ctx: &mut OperationVisitorContext<'a>,
        err_ctx: &mut ValidationErrorContext,
        fragment_spread: &FragmentSpread,
    ) {
        self.check_duplicate_directive(ctx, err_ctx, &fragment_spread.directives)
    }

    fn enter_inline_fragment(
        &mut self,
        ctx: &mut OperationVisitorContext<'a>,
        err_ctx: &mut ValidationErrorContext,
        inline_fragment: &InlineFragment,
    ) {
        self.check_duplicate_directive(ctx, err_ctx, &inline_fragment.directives)
    }
}

impl ValidationRule for UniqueDirectivesPerLocation {
    fn error_code<'a>(&self) -> &'a str {
        "UniqueDirectivesPerLocation"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut UniqueDirectivesPerLocation::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn no_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type {
            field
          }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn unique_directives_in_different_locations() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type @directiveA {
            field @directiveB
          }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn unique_directives_in_same_location() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type @directiveA @directiveB {
            field @directiveA @directiveB
          }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn same_directives_in_different_locations() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type @directiveA {
            field @directiveA
          }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn same_directives_in_similar_locations() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type {
            field @directive
            field @directive
          }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn repeatable_directives_in_same_location() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type @repeatable @repeatable {
            field @repeatable @repeatable
          }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn unknown_directives_must_be_ignored() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type @repeatable @repeatable {
            field @repeatable @repeatable
          }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn duplicate_directives_in_one_location() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type {
            field @onField @onField
          }",
        TEST_SCHEMA,
        &mut plan,
    );
    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Duplicate directive \"onField\""])
}

#[test]
fn many_duplicate_directives_in_one_location() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type {
            field @onField @onField @onField
          }",
        TEST_SCHEMA,
        &mut plan,
    );
    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec![
            "Duplicate directive \"onField\"",
            "Duplicate directive \"onField\""
        ]
    )
}

#[test]
fn different_duplicate_directives_in_one_location() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type {
            field @onField @testDirective @onField @testDirective
          }",
        TEST_SCHEMA,
        &mut plan,
    );
    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec![
            "Duplicate directive \"onField\"",
            "Duplicate directive \"testDirective\""
        ]
    )
}

#[test]
fn duplicate_directives_in_many_location() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueDirectivesPerLocation::new()));
    let errors = test_operation_with_schema(
        "fragment Test on Type @onFragmentDefinition @onFragmentDefinition {
            field @onField @onField
          }",
        TEST_SCHEMA,
        &mut plan,
    );
    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec![
            "Duplicate directive \"onFragmentDefinition\"",
            "Duplicate directive \"onField\""
        ]
    )
}
