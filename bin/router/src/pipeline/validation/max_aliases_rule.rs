use std::collections::HashMap;

use graphql_tools::{
    ast::OperationVisitorContext,
    static_graphql::query::Definition,
    validation::{
        rules::ValidationRule,
        utils::{ValidationError, ValidationErrorContext},
    },
};
use hive_router_config::limits::MaxAliasesRuleConfig;

use crate::pipeline::validation::shared::{CountableNode, VisitedFragment};

pub struct MaxAliasesRule {
    pub config: MaxAliasesRuleConfig,
}

impl ValidationRule for MaxAliasesRule {
    fn error_code<'a>(&self) -> &'a str {
        "MAX_ALIASES_EXCEEDED"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext<'_>,
        error_collector: &mut ValidationErrorContext,
    ) {
        let mut visitor = MaxAliasesVisitor {
            config: &self.config,
            visited_fragments: HashMap::with_capacity(ctx.known_fragments.len()),
            ctx,
        };
        for definition in &ctx.operation.definitions {
            let Definition::Operation(op) = definition else {
                continue;
            };

            // First start counting aliases from the operation definition
            // `op.into()` will get `CountableNode`, then `count_aliases` will
            // start counting aliases nestedly
            if let Err(err) = visitor.count_aliases(op.into()) {
                error_collector.report_error(err);
            }
        }
    }
}

struct MaxAliasesVisitor<'a, 'b> {
    config: &'b MaxAliasesRuleConfig,
    visited_fragments: HashMap<&'a str, VisitedFragment>,
    ctx: &'b OperationVisitorContext<'a>,
}

impl<'a> MaxAliasesVisitor<'a, '_> {
    fn check_limit(&self, count: usize) -> Result<usize, ValidationError> {
        if count > self.config.n {
            Err(ValidationError {
                locations: vec![],
                message: "Aliases limit exceeded.".to_string(),
                error_code: "MAX_ALIASES_EXCEEDED",
            })
        } else {
            Ok(count)
        }
    }
    fn count_aliases(
        &mut self,
        countable_node: CountableNode<'a>,
    ) -> Result<usize, ValidationError> {
        // Start with 0
        let mut alias_count: usize = 0;
        // Get the alias of the current node if it is a field
        if let CountableNode::Field(field) = countable_node {
            if field.alias.is_some() {
                alias_count = self.check_limit(alias_count + 1)?;
            }
        }

        // If it is a node that has selections, iterate over the selection set, and get their number of aliases
        if let Some(selection_set) = countable_node.selection_set() {
            for selection in &selection_set.items {
                let countable_node: CountableNode<'a> = selection.into();
                let child_aliases = self.count_aliases(countable_node)?;
                alias_count = self.check_limit(alias_count + child_aliases)?;
            }
        }

        // If it is a fragment spread, we need to count aliases of the used fragments
        if let CountableNode::FragmentSpread(node) = countable_node {
            let fragment_name = node.fragment_name.as_str();

            // Check if the fragment was already visited
            match self.visited_fragments.get(fragment_name) {
                Some(VisitedFragment::Counted(num)) => {
                    return self.check_limit(alias_count + num);
                }
                Some(VisitedFragment::Visiting) => return Ok(alias_count),
                None => {}
            }

            // If not, mark it as Visiting initially to avoid infinite loops
            self.visited_fragments
                .insert(fragment_name, VisitedFragment::Visiting);

            // If the fragment is found, get the original Fragment Definition and convert it to CountableNode
            if let Some(fragment_def) = self.ctx.known_fragments.get(fragment_name) {
                let countable_node: CountableNode<'a> = fragment_def.into();
                // Count aliases of the fragment
                let fragment_alias_count = self.count_aliases(countable_node)?;

                // Update it with the actual count
                self.visited_fragments.insert(
                    fragment_name,
                    VisitedFragment::Counted(fragment_alias_count),
                );
                alias_count = self.check_limit(alias_count + fragment_alias_count)?;
            }
        }

        Ok(alias_count)
    }
}

#[cfg(test)]
mod tests {
    use std::vec;

    use graphql_tools::{
        parser::parse_schema,
        validation::validate::{validate, ValidationPlan},
    };
    use hive_router_config::limits::MaxAliasesRuleConfig;

    use crate::pipeline::validation::max_aliases_rule::MaxAliasesRule;

    const TYPE_DEFS: &'static str = r#"
        type Book {
            title: String
            author: String
        }

        type Query {
            books: [Book]
            getBook(title: String): Book
        }
    "#;

    const QUERY: &'static str = r#"
        query {
            firstBooks: getBook(title: "null") {
                author
                title
            }
            secondBooks: getBook(title: "null") {
                author
                title
            }
        }
    "#;

    #[test]
    fn should_work_by_default() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = graphql_tools::parser::parse_query(QUERY)
            .expect("Failed to parse query")
            .into_static();
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxAliasesRule {
            config: MaxAliasesRuleConfig { n: 15 },
        })]);

        let errors = validate(&schema, &query, &validation_plan);

        assert!(errors.is_empty());
    }

    #[test]
    fn rejects_query_exceeding_max_aliases() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = graphql_tools::parser::parse_query(QUERY)
            .expect("Failed to parse query")
            .into_static();
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxAliasesRule {
            config: MaxAliasesRuleConfig { n: 1 },
        })]);

        let errors = validate(&schema, &query, &validation_plan);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].error_code, "MAX_ALIASES_EXCEEDED");
    }

    #[test]
    fn respects_fragment_aliases() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = graphql_tools::parser::parse_query(
            r#"
            query A {
                getBook(title: "null") {
                    firstTitle: title
                    ...BookFragment
                }
            }
            fragment BookFragment on Book {
                secondTitle: title
            }
        "#,
        )
        .expect("Failed to parse query")
        .into_static();
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxAliasesRule {
            config: MaxAliasesRuleConfig { n: 1 },
        })]);

        let errors = validate(&schema, &query, &validation_plan);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].error_code, "MAX_ALIASES_EXCEEDED");
    }

    #[test]
    fn do_not_crash_on_recursive_fragment() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = graphql_tools::parser::parse_query(
            r#"
            query {
                ...A
            }

            fragment A on Query {
                ...B
            }

            fragment B on Query {
                ...A
            }
        "#,
        )
        .expect("Failed to parse query")
        .into_static();
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxAliasesRule {
            config: MaxAliasesRuleConfig { n: 10 },
        })]);

        let errors = validate(&schema, &query, &validation_plan);

        assert!(errors.is_empty());
    }
}
