use std::collections::HashSet;

use graphql_tools::{
    ast::{OperationVisitor, OperationVisitorContext},
    static_graphql::query::{OperationDefinition, Selection, SelectionSet},
    validation::{
        rules::{ValidationRule, ValidationVisitor},
        utils::{ValidationError, ValidationErrorContext},
    },
};

/// Rejects every `@defer` a subscription reaches, whatever its `if` is. It only looks at the
/// document, so its result is cached with the rest of validation.
pub struct DeferInSubscriptionRule;

impl ValidationRule for DeferInSubscriptionRule {
    fn error_code(&self) -> &'static str {
        "DEFER_IN_SUBSCRIPTION"
    }

    fn visitor<'doc>(&self) -> ValidationVisitor<'doc> {
        Box::new(DeferInSubscriptionVisitor {
            walked_fragments: HashSet::new(),
        })
    }
}

struct DeferInSubscriptionVisitor {
    /// Fragments already looked at, a `@defer` in one is reported once.
    walked_fragments: HashSet<String>,
}

impl<'doc> OperationVisitor<'doc, ValidationErrorContext> for DeferInSubscriptionVisitor {
    fn enter_operation_definition(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        user_context: &mut ValidationErrorContext,
        operation: &OperationDefinition,
    ) {
        if let OperationDefinition::Subscription(subscription) = operation {
            self.walk(context, user_context, &subscription.selection_set);
        }
    }
}

impl DeferInSubscriptionVisitor {
    fn walk(
        &mut self,
        context: &OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        set: &SelectionSet,
    ) {
        for selection in &set.items {
            for directive in selection.directives() {
                if directive.name == "defer" {
                    user_context.report_error(ValidationError {
                        locations: vec![directive.position],
                        message: "`@defer` is not supported in subscriptions.".to_string(),
                        error_code: "DEFER_IN_SUBSCRIPTION",
                    });
                }
            }
            match selection {
                Selection::Field(field) => self.walk(context, user_context, &field.selection_set),
                Selection::InlineFragment(fragment) => {
                    self.walk(context, user_context, &fragment.selection_set)
                }
                Selection::FragmentSpread(spread) => {
                    if !self.walked_fragments.insert(spread.fragment_name.clone()) {
                        continue;
                    }
                    if let Some(fragment) =
                        context.known_fragments.get(spread.fragment_name.as_str())
                    {
                        self.walk(context, user_context, &fragment.selection_set);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::{parse_query, parse_schema};
    use graphql_tools::validation::validate::{validate, ValidationPlan};

    use super::DeferInSubscriptionRule;

    fn errors(query: &str) -> usize {
        let schema = parse_schema::<String>(
            "type Query { me: User } type Subscription { onUser: User } type User { a: Int b: Int }",
        )
        .unwrap()
        .into_static();
        let query = parse_query::<String>(query).unwrap().into_static();
        let plan = ValidationPlan::from(vec![Box::new(DeferInSubscriptionRule) as Box<_>]);
        validate(&schema, &query, &plan).len()
    }

    #[test]
    fn rejects_any_defer_in_a_subscription() {
        assert_eq!(errors("subscription { onUser { ... @defer { a } } }"), 1);
        // turned off or skipped, it's still rejected
        assert_eq!(
            errors("subscription ($d: Boolean!) { onUser { ... @defer(if: false) { a } ... @include(if: $d) @defer(if: $d) { b } } }"),
            2
        );
    }

    #[test]
    fn rejects_a_defer_in_a_fragment_once() {
        let query = "subscription { onUser { ...F } other: onUser { ...F } } fragment F on User { ... @defer { a } }";
        assert_eq!(errors(query), 1);
    }

    #[test]
    fn leaves_queries_alone() {
        let query = "query { me { ...F } } subscription { onUser { a } } fragment F on User { ... @defer { a } }";
        assert_eq!(errors(query), 0);
    }
}
