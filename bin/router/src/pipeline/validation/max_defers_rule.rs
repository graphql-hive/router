use std::collections::HashMap;

use crate::config::limits::MaxDefersRuleConfig;
use crate::pipeline::validation::shared::VisitedFragment;
use graphql_tools::{
    ast::{OperationVisitor, OperationVisitorContext},
    parser::Pos,
    static_graphql::query::{
        Definition, Directive, Document, FragmentDefinition, Selection, SelectionSet,
    },
    validation::{
        rules::{ValidationRule, ValidationVisitor},
        utils::{ValidationError, ValidationErrorContext},
    },
};

/// Limits how many `@defer`s an operation has, with or without `if`, counted the way they'll
/// run: a fragment's `@defer`s count once for every place it's spread. Each one is a deferred
/// part with fetches of its own, and one with `if` is planned twice, deferred and not.
pub struct MaxDefersRule {
    pub config: MaxDefersRuleConfig,
}

impl ValidationRule for MaxDefersRule {
    fn error_code(&self) -> &'static str {
        "MAX_DEFERS_EXCEEDED"
    }

    fn visitor<'doc>(&self) -> ValidationVisitor<'doc> {
        Box::new(MaxDefersVisitor {
            n: self.config.n,
            fragments: HashMap::new(),
        })
    }
}

struct MaxDefersVisitor<'doc> {
    n: usize,
    /// How many `@defer`s each fragment brings wherever it's spread, its spreads inlined.
    fragments: HashMap<&'doc str, VisitedFragment>,
}

impl<'doc> MaxDefersVisitor<'doc> {
    /// `total` plus the `@defer`s in `set`, its fragment spreads inlined. `crossed` gets where the
    /// count first went past the limit: the `@defer`, or the spread that brought it.
    fn count(
        &mut self,
        known: &HashMap<&'doc str, &'doc FragmentDefinition>,
        set: &'doc SelectionSet,
        mut total: usize,
        crossed: &mut Option<Pos>,
    ) -> usize {
        for item in &set.items {
            match item {
                Selection::Field(field) => {
                    total = self.count(known, &field.selection_set, total, crossed);
                }
                Selection::InlineFragment(fragment) => {
                    if let Some(defer) = find_defer(&fragment.directives) {
                        total = add(total, 1, defer.position, self.n, crossed);
                    }
                    total = self.count(known, &fragment.selection_set, total, crossed);
                }
                Selection::FragmentSpread(spread) => {
                    if let Some(defer) = find_defer(&spread.directives) {
                        total = add(total, 1, defer.position, self.n, crossed);
                    }
                    let inside = self.fragment(known, &spread.fragment_name);
                    total = add(total, inside, spread.position, self.n, crossed);
                }
            }
        }
        total
    }

    /// The `@defer`s a fragment brings, counted once and kept. A fragment that spreads itself
    /// isn't valid, it counts as none here.
    fn fragment(
        &mut self,
        known: &HashMap<&'doc str, &'doc FragmentDefinition>,
        name: &str,
    ) -> usize {
        let Some((&name, definition)) = known.get_key_value(name) else {
            return 0;
        };
        match self.fragments.get(name) {
            Some(VisitedFragment::Counted(count)) => return *count,
            Some(VisitedFragment::Visiting) => return 0,
            None => {}
        }
        self.fragments.insert(name, VisitedFragment::Visiting);
        let count = self.count(known, &definition.selection_set, 0, &mut None);
        self.fragments.insert(name, VisitedFragment::Counted(count));
        count
    }
}

/// `total` plus `count`, noting `at` in `crossed` when that's the first time past `n`.
/// Saturating, nested spreads multiply.
fn add(total: usize, count: usize, at: Pos, n: usize, crossed: &mut Option<Pos>) -> usize {
    let total = total.saturating_add(count);
    if crossed.is_none() && total > n {
        *crossed = Some(at);
    }
    total
}

fn find_defer(directives: &[Directive]) -> Option<&Directive> {
    directives
        .iter()
        .find(|directive| directive.name == "defer")
}

impl<'doc> OperationVisitor<'doc, ValidationErrorContext> for MaxDefersVisitor<'doc> {
    fn enter_document(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        user_context: &mut ValidationErrorContext,
        document: &'doc Document,
    ) {
        // each operation on its own, the request runs one of them
        for definition in &document.definitions {
            let Definition::Operation(operation) = definition else {
                continue;
            };
            let mut crossed = None;
            self.count(
                &context.known_fragments,
                operation.selection_set(),
                0,
                &mut crossed,
            );
            if let Some(at) = crossed {
                user_context.report_error(ValidationError {
                    locations: vec![at],
                    message: "Defer limit exceeded.".to_string(),
                    error_code: "MAX_DEFERS_EXCEEDED",
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::{parse_query, parse_schema};
    use graphql_tools::validation::validate::{validate, ValidationPlan};

    use super::MaxDefersRule;
    use crate::config::limits::MaxDefersRuleConfig;

    fn errors(n: usize, query: &str) -> Vec<String> {
        let schema =
            parse_schema::<String>("type Query { me: User } type User { a: Int b: Int c: Int }")
                .unwrap()
                .into_static();
        let query = parse_query::<String>(query).unwrap().into_static();
        let plan = ValidationPlan::from(vec![Box::new(MaxDefersRule {
            config: MaxDefersRuleConfig { n },
        }) as Box<_>]);
        validate(&schema, &query, &plan)
            .into_iter()
            .map(|error| error.message)
            .collect()
    }

    #[test]
    fn counts_every_defer_with_or_without_if() {
        let query = "query ($a: Boolean!) { me { ... @defer { a } ... @defer(if: $a) { b } ...F } } fragment F on User { ... @defer(if: false) { c } }";
        assert!(errors(3, query).is_empty());
        assert_eq!(errors(2, query), vec!["Defer limit exceeded."]);
    }

    #[test]
    fn other_directives_dont_count() {
        let query = "query ($a: Boolean!) { me { a @include(if: $a) ... @skip(if: $a) { b } ... @defer { c } } }";
        assert!(errors(1, query).is_empty());
    }

    #[test]
    fn a_fragment_counts_for_every_spread() {
        // one `@defer` in the text, three once `F` is inlined, nine through `G`
        let query = "{ me { ...F ...F ...F } } fragment F on User { ... @defer { a } }";
        assert!(errors(3, query).is_empty());
        assert_eq!(errors(2, query), vec!["Defer limit exceeded."]);
        let nested = "{ me { ...G ...G ...G } } fragment G on User { ...F ...F ...F } fragment F on User { ... @defer { a } }";
        assert!(errors(9, nested).is_empty());
        assert_eq!(errors(8, nested), vec!["Defer limit exceeded."]);
    }

    #[test]
    fn each_operation_counts_on_its_own() {
        let query = "query A { me { ... @defer { a } ... @defer { b } } } query B { me { ... @defer { c } ... @defer { a } } }";
        assert!(errors(2, query).is_empty());
        assert_eq!(errors(1, query).len(), 2);
    }

    #[test]
    fn a_defer_on_a_spread_counts_too() {
        let query = "{ me { ...F @defer } } fragment F on User { ... @defer { a } }";
        assert!(errors(2, query).is_empty());
        assert_eq!(errors(1, query), vec!["Defer limit exceeded."]);
    }
}
