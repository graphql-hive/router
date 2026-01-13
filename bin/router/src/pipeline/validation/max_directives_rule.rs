use std::collections::HashMap;

use graphql_tools::{
    ast::OperationVisitorContext,
    static_graphql::query::{Definition, Directive, OperationDefinition},
    validation::{
        rules::ValidationRule,
        utils::{ValidationError, ValidationErrorContext},
    },
};
use hive_router_config::limits::MaxDirectivesRuleConfig;

use crate::pipeline::validation::shared::CountableNode;

pub struct MaxDirectivesRule {
    pub config: MaxDirectivesRuleConfig,
}

impl ValidationRule for MaxDirectivesRule {
    fn error_code<'a>(&self) -> &'a str {
        "MAX_DIRECTIVES_EXCEEDED"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext<'_>,
        error_collector: &mut ValidationErrorContext,
    ) {
        for definition in &ctx.operation.definitions {
            if let Definition::Operation(op) = definition {
                let mut visitor = MaxDirectivesVisitor {
                    visited_fragments: HashMap::new(),
                    ctx,
                };
                // First start counting directives from the operation definition
                // `op.into()` will get `CountableNode`, then `count_directives` will
                // start counting directives nestedly
                let directives = visitor.count_directives(op.into());
                if directives > self.config.n as i32 {
                    let message = if self.config.expose_limits {
                        format!(
                            "Directives limit of {} exceeded, found {}",
                            self.config.n, directives
                        )
                    } else {
                        "Directives limit exceeded".to_string()
                    };

                    error_collector.report_error(ValidationError {
                        message,
                        locations: vec![],
                        error_code: self.error_code(),
                    });
                }
            }
        }
    }
}

struct MaxDirectivesVisitor<'a, 'b> {
    visited_fragments: HashMap<&'a str, i32>,
    ctx: &'b mut OperationVisitorContext<'a>,
}

impl<'a> CountableNode<'a> {
    fn get_directives(&self) -> Option<&'a [Directive]> {
        match self {
            CountableNode::Field(field) => Some(&field.directives),
            CountableNode::FragmentDefinition(fragment_def) => Some(&fragment_def.directives),
            CountableNode::InlineFragment(inline_fragment) => Some(&inline_fragment.directives),
            CountableNode::OperationDefinition(op_def) => match op_def {
                OperationDefinition::Query(query) => Some(&query.directives),
                OperationDefinition::Mutation(mutation) => Some(&mutation.directives),
                OperationDefinition::Subscription(subscription) => Some(&subscription.directives),
                OperationDefinition::SelectionSet(_) => None,
            },
            CountableNode::FragmentSpread(fragment_spread) => Some(&fragment_spread.directives),
        }
    }
}

impl<'a> MaxDirectivesVisitor<'a, '_> {
    fn count_directives(&mut self, countable_node: CountableNode<'a>) -> i32 {
        // Start with 0
        let mut directive_cnt: i32 = 0;
        // Get the directives of the current node
        if let Some(directives) = countable_node.get_directives() {
            directive_cnt += directives.len() as i32;
        }

        // If it is a node that has selections, iterate over the selection set, and get their number of directives
        if let Some(selection_set) = countable_node.selection_set() {
            for selection in &selection_set.items {
                let countable_node: CountableNode<'a> = selection.into();
                directive_cnt += self.count_directives(countable_node);
            }
        }

        // If it is a fragment spread, we need to count directives of the used fragments
        if let CountableNode::FragmentSpread(countable_node) = countable_node {
            let fragment_name = countable_node.fragment_name.as_str();
            // If it is already visited, add it to the total
            if let Some(visited_fragment_cnt) = self.visited_fragments.get(fragment_name) {
                // Only if it is a positive integer, add it
                // Because it might be -1 because of the logic below
                if *visited_fragment_cnt > 0 {
                    directive_cnt += visited_fragment_cnt;
                }
            } else {
                // If not, let's mark it as -1 initially to avoid infinite loops,
                // because fragments can refer itself recursively at some point.
                // See the tests at the bottom of this file to understand the use cases fully.
                self.visited_fragments.insert(fragment_name, -1);

                // If the fragment is found, get the original Fragment Definition and convert it to CountableNode
                if let Some(fragment_def) = self.ctx.known_fragments.get(fragment_name) {
                    let countable_node: CountableNode<'a> = fragment_def.into();
                    // Count directives of the fragment
                    let fragment_directive_cnt = self.count_directives(countable_node);
                    // If it was marked as -1, it means the fragment is used recursively
                    // So we need to set it now
                    if self.visited_fragments.get(fragment_name) == Some(&-1) {
                        self.visited_fragments
                            .insert(fragment_name, fragment_directive_cnt);
                    }
                    directive_cnt += fragment_directive_cnt;
                }
            }
        }

        directive_cnt
    }
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::{parse_query, parse_schema};
    use graphql_tools::validation::validate::{validate, ValidationPlan};
    use hive_router_config::limits::MaxDirectivesRuleConfig;

    use crate::pipeline::validation::max_directives_rule::MaxDirectivesRule;

    const TYPE_DEFS: &'static str = r#"
  type Book {
    title: String
    author: String
  }

  type Query {
    books: [Book]
  }
"#;

    const QUERY: &'static str = r#"
  query {
    __typename @a @a @a @a
  }
"#;

    #[test]
    fn works() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = parse_query(QUERY)
            .expect("Failed to parse query")
            .into_static();
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDirectivesRule {
            config: MaxDirectivesRuleConfig::default(),
        })]);

        let errors = validate(&schema, &query, &validation_plan);

        assert!(errors.is_empty());
    }

    #[test]
    fn rejects_query_exceeding_max_directives() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = parse_query(QUERY)
            .expect("Failed to parse query")
            .into_static();
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDirectivesRule {
            config: MaxDirectivesRuleConfig {
                n: 3,
                expose_limits: true,
            },
        })]);

        let errors = validate(&schema, &query, &validation_plan);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Directives limit of 3 exceeded, found 4");
    }

    #[test]
    fn works_on_fragment() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = parse_query(
            r#"
        query {
        ...DirectivesFragment
      }

      fragment DirectivesFragment on Query {
        __typename @a @a @a @a
      }
    "#,
        )
        .expect("Failed to parse query")
        .into_static();

        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDirectivesRule {
            config: MaxDirectivesRuleConfig {
                n: 3,
                expose_limits: true,
            },
        })]);

        let errors = validate(&schema, &query, &validation_plan);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Directives limit of 3 exceeded, found 4");
    }

    #[test]
    fn not_crash_on_recursive_fragment() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = parse_query(
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

        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDirectivesRule {
            config: MaxDirectivesRuleConfig::default(),
        })]);

        let errors = validate(&schema, &query, &validation_plan);

        assert!(errors.is_empty());
    }

    #[test]
    fn rejects_with_a_generic_message_when_expose_limits_is_false() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = parse_query(QUERY)
            .expect("Failed to parse query")
            .into_static();
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDirectivesRule {
            config: MaxDirectivesRuleConfig {
                n: 3,
                expose_limits: false,
            },
        })]);

        let errors = validate(&schema, &query, &validation_plan);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Directives limit exceeded");
    }

    #[test]
    fn rejects_with_detailed_error_message_when_expose_limits_is_true() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = parse_query(QUERY)
            .expect("Failed to parse query")
            .into_static();
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDirectivesRule {
            config: MaxDirectivesRuleConfig {
                n: 3,
                expose_limits: true,
            },
        })]);

        let errors = validate(&schema, &query, &validation_plan);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Directives limit of 3 exceeded, found 4");
    }

    #[test]
    fn count_directives_on_recursive_fragment_spreads() {
        let schema = parse_schema(TYPE_DEFS)
            .expect("Failed to parse schema")
            .into_static();
        let query = parse_query(
            r#"
        query {
          ...A
        }
        fragment A on Query {
          ...B @directive1 @directive2
        }
        fragment B on Query {
          ...A @directive3 @directive4
        }
      "#,
        )
        .expect("Failed to parse query")
        .into_static();
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDirectivesRule {
            config: MaxDirectivesRuleConfig {
                n: 1,
                expose_limits: true,
            },
        })]);
        let errors = validate(&schema, &query, &validation_plan);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Directives limit of 1 exceeded, found 4");
    }
}
