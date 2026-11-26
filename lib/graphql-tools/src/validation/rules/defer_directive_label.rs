use std::collections::HashMap;

use super::ValidationRule;
use crate::ast::{OperationVisitor, OperationVisitorContext};
use crate::parser::Pos;
use crate::static_graphql::query::{Directive, Value};
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Defer directive labels
///
/// A `label` on `@defer` has to be a static string, and unique across every `@defer` in the
/// document.
///
/// See https://github.com/graphql/graphql-js/blob/v17.1.0/src/validation/rules/DeferDirectiveLabelRule.ts
#[derive(Default)]
pub struct DeferDirectiveLabel {
    known_labels: HashMap<String, Pos>,
}

impl DeferDirectiveLabel {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for DeferDirectiveLabel {
    fn enter_directive(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        directive: &Directive,
    ) {
        if directive.name != "defer" {
            return;
        }
        let Some((_, value)) = directive.arguments.iter().find(|(name, _)| name == "label") else {
            return;
        };
        match value {
            Value::Null => {}
            Value::String(label) => match self.known_labels.get(label) {
                Some(known) => user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    locations: vec![*known, directive.position],
                    message: "Value for argument \"defer(label:)\" must be unique across all Defer directive usages.".to_string(),
                }),
                None => {
                    self.known_labels.insert(label.clone(), directive.position);
                }
            },
            _ => user_context.report_error(ValidationError {
                error_code: self.error_code(),
                locations: vec![directive.position],
                message: "Argument \"@defer(label:)\" must be a static string.".to_string(),
            }),
        }
    }
}

impl ValidationRule for DeferDirectiveLabel {
    fn error_code(&self) -> &'static str {
        "DeferDirectiveLabel"
    }

    fn visitor<'doc>(&self) -> super::ValidationVisitor<'doc> {
        Box::new(DeferDirectiveLabel::new())
    }
}

#[cfg(test)]
static DEFER_TEST_SCHEMA: &str = "
directive @defer(if: Boolean! = true, label: String) on FRAGMENT_SPREAD | INLINE_FRAGMENT
type Query { me: User }
type User { id: ID name: String }
";

#[cfg(test)]
fn messages(operation: &str) -> Vec<String> {
    use crate::validation::test_utils::*;

    let plan = create_plan_from_rule(Box::new(DeferDirectiveLabel::new()));
    test_operation_with_schema(operation, DEFER_TEST_SCHEMA, &plan)
        .into_iter()
        .map(|error| error.message)
        .collect()
}

#[test]
fn unique_static_labels_are_valid() {
    assert!(messages(
        r#"{ me { ... @defer(label: "a") { id } ... @defer(label: "b") { name } } }"#
    )
    .is_empty());
}

#[test]
fn missing_and_null_labels_are_valid() {
    assert!(
        messages(r#"{ me { ... @defer { id } ... @defer(label: null) { name } } }"#).is_empty()
    );
}

#[test]
fn a_fragment_spread_twice_counts_once() {
    assert!(messages(
        r#"{ me { ...F } other: me { ...F } } fragment F on User { ... @defer(label: "a") { id } }"#
    )
    .is_empty());
}

#[test]
fn duplicate_labels_are_invalid() {
    assert_eq!(
        messages(r#"{ me { ... @defer(label: "a") { id } ... @defer(label: "a") { name } } }"#),
        vec!["Value for argument \"defer(label:)\" must be unique across all Defer directive usages."]
    );
}

#[test]
fn labels_are_unique_across_operations() {
    assert_eq!(
        messages(
            r#"query A { me { ... @defer(label: "a") { id } } } query B { me { ... @defer(label: "a") { id } } }"#
        )
        .len(),
        1
    );
}

#[test]
fn variable_labels_are_invalid() {
    assert_eq!(
        messages(r#"query ($l: String) { me { ... @defer(label: $l) { id } } }"#),
        vec!["Argument \"@defer(label:)\" must be a static string."]
    );
}
