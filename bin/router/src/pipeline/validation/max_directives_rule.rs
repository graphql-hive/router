use std::collections::HashMap;

use graphql_tools::{
    ast::OperationVisitorContext,
    static_graphql::query::Definition,
    validation::{
        rules::ValidationRule,
        utils::{ValidationError, ValidationErrorContext},
    },
};
use hive_router_config::limits::MaxDirectivesRuleConfig;

use crate::pipeline::validation::shared::{CountableNode, VisitedFragment};

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
            let Definition::Operation(op) = definition else {
                continue;
            };

            let mut visitor = MaxDirectivesVisitor {
                visited_fragments: HashMap::new(),
                ctx,
            };
            // First start counting directives from the operation definition
            // `op.into()` will get `CountableNode`, then `count_directives` will
            // start counting directives nestedly
            let directives = visitor.count_directives(op.into());

            if directives <= self.config.n {
                continue;
            }

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

struct MaxDirectivesVisitor<'a, 'b> {
    visited_fragments: HashMap<&'a str, VisitedFragment>,
    ctx: &'b mut OperationVisitorContext<'a>,
}

impl<'a> MaxDirectivesVisitor<'a, '_> {
    fn count_directives(&mut self, countable_node: CountableNode<'a>) -> usize {
        // Start with 0
        let mut directive_count: usize = 0;
        // Get the directives of the current node
        if let Some(directives) = countable_node.get_directives() {
            directive_count += directives.len();
        }

        // If it is a node that has selections, iterate over the selection set, and get their number of directives
        if let Some(selection_set) = countable_node.selection_set() {
            for selection in &selection_set.items {
                let countable_node: CountableNode<'a> = selection.into();
                directive_count += self.count_directives(countable_node);
            }
        }

        // If it is a fragment spread, we need to count directives of the used fragments
        if let CountableNode::FragmentSpread(node) = countable_node {
            let fragment_name = node.fragment_name.as_str();

            // Check if the fragment was already visited
            match self.visited_fragments.get(fragment_name) {
                Some(VisitedFragment::Counted(num)) => {
                    return directive_count + num;
                }
                Some(VisitedFragment::Visiting) => return directive_count,
                None => {}
            }

            // If not, mark it as Visiting initially to avoid infinite loops
            self.visited_fragments
                .insert(fragment_name, VisitedFragment::Visiting);

            // If the fragment is found, get the original Fragment Definition and convert it to CountableNode
            if let Some(fragment_def) = self.ctx.known_fragments.get(fragment_name) {
                let countable_node: CountableNode<'a> = fragment_def.into();
                // Count directives of the fragment
                let fragment_directive_count = self.count_directives(countable_node);

                // Update it with the actual count
                self.visited_fragments.insert(
                    fragment_name,
                    VisitedFragment::Counted(fragment_directive_count),
                );
                directive_count += fragment_directive_count;
            }
        }

        directive_count
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
                expose_limits: false,
            },
        })]);
        let errors = validate(&schema, &query, &validation_plan);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Directives limit exceeded");
    }
}
