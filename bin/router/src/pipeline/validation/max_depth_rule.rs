use std::{cmp, collections::HashMap};

use graphql_tools::{
    ast::OperationVisitorContext,
    static_graphql::query::{Definition, Selection},
    validation::{
        rules::ValidationRule,
        utils::{ValidationError, ValidationErrorContext},
    },
};
use hive_router_config::limits::MaxDepthRuleConfig;

use crate::pipeline::validation::shared::{CountableNode, VisitedFragment};

pub struct MaxDepthRule {
    pub config: MaxDepthRuleConfig,
}

impl ValidationRule for MaxDepthRule {
    fn error_code<'a>(&self) -> &'a str {
        "MAX_DEPTH_EXCEEDED"
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

            let mut visitor = MaxDepthVisitor {
                config: &self.config,
                visited_fragments: HashMap::new(),
                ctx,
            };
            let depth = visitor.count_depth(op.into(), None);

            if depth <= self.config.n {
                continue;
            }

            let message = if self.config.expose_limits {
                format!(
                    "Query depth limit of {} exceeded, found {}.",
                    self.config.n, depth
                )
            } else {
                "Query depth limit exceeded.".to_string()
            };

            error_collector.report_error(ValidationError {
                message,
                locations: vec![],
                error_code: "MAX_DEPTH_EXCEEDED",
            });
        }
    }
}

struct MaxDepthVisitor<'a, 'b> {
    config: &'b MaxDepthRuleConfig,
    visited_fragments: HashMap<&'a str, VisitedFragment>,
    ctx: &'b mut OperationVisitorContext<'a>,
}

impl<'a> MaxDepthVisitor<'a, '_> {
    fn count_depth(&mut self, node: CountableNode<'a>, parent_depth: Option<usize>) -> usize {
        // If introspection queries are to be ignored, skip them from the root
        if self.config.ignore_introspection {
            if let CountableNode::Field(field) = node {
                let field_name = field.name.as_str();
                if field_name == "__schema" || field_name == "__type" {
                    return 0;
                }
            }
        }

        // Initialize parent depth
        let mut parent_depth = parent_depth.unwrap_or(0);

        // Current depth starts as parent depth
        let mut depth = parent_depth;

        // Traverse the selection set if present
        if let Some(selection_set) = node.selection_set() {
            for child in &selection_set.items {
                // Decide whether to increase depth based on flatten_fragments config
                let increase_by = if self.config.flatten_fragments
                    && matches!(
                        child,
                        Selection::FragmentSpread(_) | Selection::InlineFragment(_)
                    ) {
                    0
                } else {
                    1
                };

                depth = cmp::max(
                    depth,
                    self.count_depth(child.into(), Some(parent_depth + increase_by)),
                );
            }
        }

        // If the node is a FragmentSpread, handle fragment depth counting
        if let CountableNode::FragmentSpread(node) = node {
            // If flatten_fragments is false, increase parent depth
            // for the fragment spread itself
            if !self.config.flatten_fragments {
                parent_depth += 1;
            }

            let fragment_name = node.fragment_name.as_str();
            // Find if the fragment was already visited
            let visited_fragment = self.visited_fragments.get(fragment_name);
            if let Some(visited_fragment_depth) = visited_fragment {
                if let VisitedFragment::Counted(visited_fragment_depth) = visited_fragment_depth {
                    // If it was already visited, return the cached depth
                    return parent_depth + visited_fragment_depth;
                }
            } else {
                // If not, mark it as Visiting initially to avoid infinite loops,
                // because fragments can refer itself recursively at some point.
                // See the tests at the bottom of this file to understand the use cases fully.
                self.visited_fragments
                    .insert(fragment_name, VisitedFragment::Visiting);
                // Look up the fragment definition by its name
                let fragment = self.ctx.known_fragments.get(fragment_name);
                if let Some(fragment) = fragment {
                    // Count the depth of the fragment
                    let fragment_depth = self.count_depth(fragment.into(), Some(0));

                    // Update it with the actual depth.
                    self.visited_fragments
                        .insert(fragment_name, VisitedFragment::Counted(fragment_depth));

                    // Update the overall depth
                    depth = cmp::max(depth, parent_depth + fragment_depth);
                }
            }
        }

        depth
    }
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::{parse_query, parse_schema};
    use graphql_tools::validation::validate::ValidationPlan;
    use hive_router_config::limits::MaxDepthRuleConfig;

    use crate::pipeline::validation::max_depth_rule::MaxDepthRule;

    const TYPE_DEFS: &'static str = r#"
        type Author {
            name: String
            books: [Book]
        }

        type Book {
            title: String
            author: Author
        }

        type Query {
            books: [Book]
        }
    "#;

    const QUERY: &'static str = r#"
            query {
                books {
                    author {
                        name
                    }
                    title
                }
            }
        "#;

    #[test]
    fn works() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig::default(),
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document =
            parse_query(QUERY).expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(errors.is_empty());
    }

    #[test]
    fn rejects_query_exceeding_max_depth() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 1,
                ..Default::default()
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document =
            parse_query(QUERY).expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            !errors.is_empty(),
            "Expected validation errors but found none"
        );

        let error = &errors[0];
        assert_eq!(error.message, "Query depth limit of 1 exceeded, found 3.");
    }

    #[test]
    fn rejects_fragment_exceeding_max_depth() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 4,
                ..Default::default()
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document = parse_query(
            r#"
      query {
        ...BooksFragment
      }

      fragment BooksFragment on Query {
        books {
          title
          author {
            name
          }
        }
      }
        "#,
        )
        .expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            !errors.is_empty(),
            "Expected validation errors but found none"
        );

        let error = &errors[0];
        assert_eq!(error.message, "Query depth limit of 4 exceeded, found 5.");
    }

    #[test]
    fn rejects_flattened_fragment_exceeding_max_depth() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 2,
                flatten_fragments: true,
                ..Default::default()
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document = parse_query(
            r#"
      query {
        ...BooksFragment
      }

      fragment BooksFragment on Query {
        books {
          title
          author {
            name
          }
        }
      }
        "#,
        )
        .expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            !errors.is_empty(),
            "Expected validation errors but found none"
        );

        let error = &errors[0];
        assert_eq!(error.message, "Query depth limit of 2 exceeded, found 3.");
    }

    #[test]
    fn rejects_flattened_inline_fragment_exceeding_max_depth() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 2,
                flatten_fragments: true,
                ..Default::default()
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document = parse_query(
            r#"
      query {
        ... on Query {
          books {
            title
            author {
              name
            }
          }
        }
      }
        "#,
        )
        .expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            !errors.is_empty(),
            "Expected validation errors but found none"
        );

        let error = &errors[0];
        assert_eq!(error.message, "Query depth limit of 2 exceeded, found 3.");
    }

    const INTROSPECTION_QUERY: &'static str =
        include_str!("test_fixtures/introspection_query.fixture.graphql");
    #[test]
    fn allows_introspection_queries_when_ignored() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 2,
                ignore_introspection: true,
                ..Default::default()
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc = parse_query(INTROSPECTION_QUERY).expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            errors.is_empty(),
            "Expected no validation errors but found some"
        );
    }

    #[test]
    fn rejects_recursive_fragment_exceeding_max_depth() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 3,
                ..Default::default()
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document = parse_query(
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
        .expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            !errors.is_empty(),
            "Expected validation errors but found none"
        );

        let error = &errors[0];
        assert_eq!(error.message, "Query depth limit of 3 exceeded, found 5.");
    }

    #[test]
    fn rejects_with_a_generic_message_when_expose_limits_is_false() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 2,
                expose_limits: false,
                ..Default::default()
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document =
            parse_query(QUERY).expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            !errors.is_empty(),
            "Expected validation errors but found none"
        );

        let error = &errors[0];
        assert_eq!(error.message, "Query depth limit exceeded.");
    }

    #[test]
    fn rejects_with_detailed_error_message_when_expose_limits_is_true() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 2,
                expose_limits: true,
                ..Default::default()
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document =
            parse_query(QUERY).expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            !errors.is_empty(),
            "Expected validation errors but found none"
        );

        let error = &errors[0];
        assert_eq!(error.message, "Query depth limit of 2 exceeded, found 3.");
    }

    #[test]
    fn rejects_for_fragment_named_schema_exceeding_max_depth() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 6,
                expose_limits: true,
                ..Default::default()
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document = parse_query(
            r#"
      query {
        books {
          author {
            books {
              author {
                ...__schema
              }
            }
          }
        }
      }
      fragment __schema on Author {
        books {
          title
        }
      }
        "#,
        )
        .expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            !errors.is_empty(),
            "Expected validation errors but found none"
        );

        let error = &errors[0];
        assert_eq!(error.message, "Query depth limit of 6 exceeded, found 8.");
    }

    #[test]
    fn rejects_for_exceeding_max_depth_by_reusing_a_cached_fragment() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 6,
                expose_limits: true,
                ..Default::default()
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document = parse_query(
            r#"
      query {
        books {
          author {
            ...Test
          }
        }
        books {
          author {
            books {
              author {
                ...Test
              }
            }
          }
        }
      }
      fragment Test on Author {
        books {
          title
        }
      }
        "#,
        )
        .expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            !errors.is_empty(),
            "Expected validation errors but found none"
        );

        let error = &errors[0];
        assert_eq!(error.message, "Query depth limit of 6 exceeded, found 8.");
    }
}
