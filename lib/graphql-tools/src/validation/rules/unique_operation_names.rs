use std::collections::HashMap;

use super::ValidationRule;
use crate::ast::{visit_document, AstNodeWithName, OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::*;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Unique operation names
///
/// A GraphQL document is only valid if all defined operations have unique names.
///
/// See https://spec.graphql.org/draft/#sec-Operation-Name-Uniqueness
pub struct UniqueOperationNames<'a> {
    findings_counter: HashMap<&'a str, i32>,
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for UniqueOperationNames<'a> {
    fn enter_operation_definition(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        operation_definition: &'a OperationDefinition,
    ) {
        if let Some(name) = operation_definition.node_name() {
            self.store_finding(name);
        }
    }
}

impl<'a> Default for UniqueOperationNames<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> UniqueOperationNames<'a> {
    pub fn new() -> Self {
        Self {
            findings_counter: HashMap::new(),
        }
    }

    fn store_finding(&mut self, name: &'a str) {
        let value = *self.findings_counter.entry(name).or_insert(0);
        self.findings_counter.insert(name, value + 1);
    }
}

impl<'u> ValidationRule for UniqueOperationNames<'u> {
    fn error_code<'a>(&self) -> &'a str {
        "UniqueOperationNames"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        let mut rule = UniqueOperationNames::new();

        visit_document(&mut rule, ctx.operation, ctx, error_collector);

        rule.findings_counter
            .into_iter()
            .filter(|(_key, value)| *value > 1)
            .for_each(|(key, _value)| {
                error_collector.report_error(ValidationError {
                    error_code: self.error_code(),
                    message: format!("There can be only one operation named \"{}\".", key),
                    locations: vec![],
                })
            })
    }
}

#[test]
fn no_operations() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueOperationNames::new()));
    let errors = test_operation_with_schema(
        "fragment fragA on Type {
          field
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn one_anon_operation() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueOperationNames::new()));
    let errors = test_operation_with_schema(
        "{
          field
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn one_named_operation() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueOperationNames::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          field
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn multiple_operations() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueOperationNames::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          field
        }
        query Bar {
          field
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn multiple_operations_of_different_types() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueOperationNames::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          field
        }
        mutation Bar {
          field
        }
        subscription Baz {
          field
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn fragment_and_operation_named_the_same() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueOperationNames::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          ...Foo
        }
        fragment Foo on Type {
          field
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn multiple_operations_of_same_name() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueOperationNames::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          fieldA
        }
        query Foo {
          fieldB
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["There can be only one operation named \"Foo\".",]
    );
}

#[test]
fn multiple_ops_of_same_name_of_different_types_mutation() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueOperationNames::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          fieldA
        }
        mutation Foo {
          fieldB
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["There can be only one operation named \"Foo\".",]
    );
}

#[test]
fn multiple_ops_of_same_name_of_different_types_subscription() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(UniqueOperationNames::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          fieldA
        }
        subscription Foo {
          fieldB
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["There can be only one operation named \"Foo\".",]
    );
}
