use std::collections::{HashMap, HashSet};

use super::ValidationRule;
use crate::ast::{OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::{
    Directive, FragmentDefinition, OperationDefinition, Selection, SelectionSet,
};
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Defer directive on root field
///
/// `@defer` can't be used on the root selection set of a mutation or a subscription. Fragments
/// sitting right on the root count too.
///
/// See https://github.com/graphql/graphql-js/blob/v17.1.0/src/validation/rules/DeferDirectiveOnRootFieldRule.ts
#[derive(Default)]
pub struct DeferDirectiveOnRootField {
    checked_fragments: HashSet<String>,
}

impl DeferDirectiveOnRootField {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for DeferDirectiveOnRootField {
    fn enter_operation_definition(
        &mut self,
        context: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        operation: &'a OperationDefinition,
    ) {
        let (operation_type, root_type) = match operation {
            OperationDefinition::Mutation(_) => ("mutation", context.schema.mutation_type()),
            OperationDefinition::Subscription(_) => {
                ("subscription", context.schema.subscription_type())
            }
            _ => return,
        };
        let Some(root_type) = root_type else {
            return;
        };
        let mut walk = Walk {
            fragments: &context.known_fragments,
            checked_fragments: &self.checked_fragments,
            visited_fragments: HashSet::new(),
            operation_type,
            root_type: &root_type.name,
            errors: user_context,
            error_code: self.error_code(),
        };
        if walk.selection_set(operation.selection_set()) {
            let visited = walk.visited_fragments;
            self.checked_fragments.extend(visited);
        }
    }
}

struct Walk<'a, 'w> {
    fragments: &'w HashMap<&'a str, &'a FragmentDefinition>,
    checked_fragments: &'w HashSet<String>,
    visited_fragments: HashSet<String>,
    operation_type: &'static str,
    root_type: &'w str,
    errors: &'w mut ValidationErrorContext,
    error_code: &'static str,
}

impl Walk<'_, '_> {
    /// Returns whether no error was found.
    fn selection_set(&mut self, set: &SelectionSet) -> bool {
        let mut valid = true;
        for selection in &set.items {
            match selection {
                Selection::Field(_) => {}
                Selection::FragmentSpread(spread) => {
                    // checked before the visited check, every spread can have its own `@defer`
                    if let Some(defer) = find(&spread.directives, "defer") {
                        valid = false;
                        self.report(defer);
                    }
                    if !self.visited_fragments.insert(spread.fragment_name.clone()) {
                        continue;
                    }
                    let Some(fragment) = self.fragments.get(spread.fragment_name.as_str()) else {
                        continue;
                    };
                    if !self.checked_fragments.contains(&spread.fragment_name) {
                        valid = self.selection_set(&fragment.selection_set) && valid;
                    }
                }
                Selection::InlineFragment(fragment) => {
                    if let Some(defer) = find(&fragment.directives, "defer") {
                        valid = false;
                        self.report(defer);
                    }
                    valid = self.selection_set(&fragment.selection_set) && valid;
                }
            }
        }
        valid
    }

    fn report(&mut self, directive: &Directive) {
        self.errors.report_error(ValidationError {
            error_code: self.error_code,
            locations: vec![directive.position],
            message: format!(
                "Defer directive cannot be used on root {} type \"{}\".",
                self.operation_type, self.root_type
            ),
        });
    }
}

fn find<'d>(directives: &'d [Directive], name: &str) -> Option<&'d Directive> {
    directives.iter().find(|directive| directive.name == name)
}

impl ValidationRule for DeferDirectiveOnRootField {
    fn error_code(&self) -> &'static str {
        "DeferDirectiveOnRootField"
    }

    fn visitor<'doc>(&self) -> super::ValidationVisitor<'doc> {
        Box::new(DeferDirectiveOnRootField::new())
    }
}

#[cfg(test)]
static DEFER_TEST_SCHEMA: &str = "
directive @defer(if: Boolean! = true, label: String) on FRAGMENT_SPREAD | INLINE_FRAGMENT
type Query { me: User }
type Mutation { rename(name: String!): User }
type Subscription { onUser: User }
type User { id: ID name: String }
";

#[cfg(test)]
fn messages(operation: &str) -> Vec<String> {
    use crate::validation::test_utils::*;

    let plan = create_plan_from_rule(Box::new(DeferDirectiveOnRootField::new()));
    test_operation_with_schema(operation, DEFER_TEST_SCHEMA, &plan)
        .into_iter()
        .map(|error| error.message)
        .collect()
}

#[test]
fn defer_on_the_query_root_is_valid() {
    assert!(messages("{ ... @defer { me { id } } }").is_empty());
}

#[test]
fn defer_below_the_mutation_root_is_valid() {
    assert!(messages("mutation { rename(name: \"x\") { id ... @defer { name } } }").is_empty());
}

#[test]
fn defer_on_the_mutation_root_is_invalid() {
    assert_eq!(
        messages("mutation { ... @defer { rename(name: \"x\") { id } } }"),
        vec!["Defer directive cannot be used on root mutation type \"Mutation\"."]
    );
}

#[test]
fn defer_in_a_fragment_on_the_mutation_root_is_invalid() {
    assert_eq!(
        messages(
            "mutation { ...F } fragment F on Mutation { ... @defer { rename(name: \"x\") { id } } }"
        ),
        vec!["Defer directive cannot be used on root mutation type \"Mutation\"."]
    );
    assert_eq!(
        messages("mutation { ...F @defer } fragment F on Mutation { rename(name: \"x\") { id } }"),
        vec!["Defer directive cannot be used on root mutation type \"Mutation\"."]
    );
}

#[test]
fn defer_on_the_subscription_root_is_invalid() {
    assert_eq!(
        messages("subscription { ... @defer(if: false) { onUser { id } } }"),
        vec!["Defer directive cannot be used on root subscription type \"Subscription\"."]
    );
}

#[test]
fn defer_on_a_second_spread_of_the_same_fragment_is_invalid() {
    assert_eq!(
        messages(
            "mutation { ...F ...F @defer } fragment F on Mutation { rename(name: \"x\") { id } }"
        ),
        vec!["Defer directive cannot be used on root mutation type \"Mutation\"."]
    );
}
