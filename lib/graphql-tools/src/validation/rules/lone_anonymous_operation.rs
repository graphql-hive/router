use super::ValidationRule;
use crate::ast::{visit_document, OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::*;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Lone Anonymous Operation
///
/// A GraphQL document is only valid if when it contains an anonymous operation
/// (the query short-hand) that it contains only that one operation definition.
///
/// https://spec.graphql.org/draft/#sec-Lone-Anonymous-Operation
pub struct LoneAnonymousOperation;

impl Default for LoneAnonymousOperation {
    fn default() -> Self {
        Self::new()
    }
}

impl LoneAnonymousOperation {
    pub fn new() -> Self {
        LoneAnonymousOperation
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for LoneAnonymousOperation {
    fn enter_document(
        &mut self,
        _: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        document: &Document,
    ) {
        let operations_count = document
            .definitions
            .iter()
            .filter(|n| {
                matches!(
                    n,
                    Definition::Operation(OperationDefinition::SelectionSet(_))
                        | Definition::Operation(OperationDefinition::Query(_))
                        | Definition::Operation(OperationDefinition::Mutation(_))
                        | Definition::Operation(OperationDefinition::Subscription(_))
                )
            })
            .count();

        for definition in &document.definitions {
            match definition {
                Definition::Operation(OperationDefinition::SelectionSet(_)) => {
                    if operations_count > 1 {
                        user_context.report_error(ValidationError {
                            error_code: self.error_code(),
                            message: "This anonymous operation must be the only defined operation."
                                .to_string(),
                            locations: vec![],
                        })
                    }
                }
                Definition::Operation(OperationDefinition::Query(query)) => {
                    if query.name.is_none() && operations_count > 1 {
                        user_context.report_error(ValidationError {
                            error_code: self.error_code(),
                            message: "This anonymous operation must be the only defined operation."
                                .to_string(),
                            locations: vec![query.position],
                        })
                    }
                }
                Definition::Operation(OperationDefinition::Mutation(mutation)) => {
                    if mutation.name.is_none() && operations_count > 1 {
                        user_context.report_error(ValidationError {
                            error_code: self.error_code(),
                            message: "This anonymous operation must be the only defined operation."
                                .to_string(),
                            locations: vec![mutation.position],
                        })
                    }
                }
                Definition::Operation(OperationDefinition::Subscription(subscription)) => {
                    if subscription.name.is_none() && operations_count > 1 {
                        user_context.report_error(ValidationError {
                            error_code: self.error_code(),
                            message: "This anonymous operation must be the only defined operation."
                                .to_string(),
                            locations: vec![subscription.position],
                        })
                    }
                }
                _ => {}
            };
        }
    }
}

impl ValidationRule for LoneAnonymousOperation {
    fn error_code<'a>(&self) -> &'a str {
        "LoneAnonymousOperation"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut LoneAnonymousOperation::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn no_operations() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LoneAnonymousOperation {}));
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

    let mut plan = create_plan_from_rule(Box::new(LoneAnonymousOperation {}));
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
fn mutiple_named() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LoneAnonymousOperation {}));
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

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn anon_operation_with_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LoneAnonymousOperation {}));
    let errors = test_operation_with_schema(
        "{
          ...Foo
        }
        fragment Foo on Type {
          field
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn multiple_anon_operations() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LoneAnonymousOperation {}));
    let errors = test_operation_with_schema(
        "{
          fieldA
        }
        {
          fieldB
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec![
            "This anonymous operation must be the only defined operation.",
            "This anonymous operation must be the only defined operation."
        ]
    );
}

#[test]
fn anon_operation_with_mutation() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LoneAnonymousOperation {}));
    let errors = test_operation_with_schema(
        "{
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
        vec!["This anonymous operation must be the only defined operation."]
    );
}

#[test]
fn anon_operation_with_subscription() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(LoneAnonymousOperation {}));
    let errors = test_operation_with_schema(
        "{
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
        vec!["This anonymous operation must be the only defined operation."]
    );
}
