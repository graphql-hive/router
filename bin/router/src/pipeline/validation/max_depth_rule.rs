use std::{cmp, collections::HashMap};

use graphql_tools::{
    ast::OperationVisitorContext,
    static_graphql::query::{Definition, Selection},
    validation::{
        rules::ValidationRule,
        utils::{ValidationError, ValidationErrorContext},
    },
};
use hive_router_config::query_complexity::MaxDepthRuleConfig;

use crate::pipeline::validation::shared::CountableNode;

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
            if let Definition::Operation(op) = definition {
                let mut visitor = MaxDepthRuleVisitor {
                    config: &self.config,
                    visited_fragments: HashMap::new(),
                    ctx,
                };
                let depth = visitor.count_depth(op.into(), None);
                if depth > self.config.n as i32 {
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
    }
}

struct MaxDepthRuleVisitor<'a, 'b> {
    config: &'a MaxDepthRuleConfig,
    visited_fragments: HashMap<String, i32>,
    ctx: &'a mut OperationVisitorContext<'b>,
}

impl MaxDepthRuleVisitor<'_, '_> {
    fn count_depth(&mut self, node: CountableNode, parent_depth: Option<i32>) -> i32 {
        if self.config.ignore_introspection {
            if let CountableNode::Field(field) = node {
                if field.name == "__schema" {
                    return 0;
                }
            }
        }

        let mut parent_depth = parent_depth.unwrap_or(0);

        let mut depth = parent_depth;

        if let Some(selection_set) = node.selection_set() {
            for child in &selection_set.items {
                if self.config.flatten_fragments
                    && (matches!(child, Selection::FragmentSpread(_))
                        || matches!(child, Selection::InlineFragment(_)))
                {
                    depth = cmp::max(depth, self.count_depth(child.into(), Some(parent_depth)));
                } else {
                    depth = cmp::max(
                        depth,
                        self.count_depth(child.into(), Some(parent_depth + 1)),
                    );
                }
            }
        }

        if let CountableNode::FragmentSpread(node) = node {
            if !self.config.flatten_fragments {
                parent_depth += 1;
            }

            let visited_fragment = self.visited_fragments.get(&node.fragment_name);
            if let Some(visited_fragment_depth) = visited_fragment {
                return parent_depth + visited_fragment_depth;
            } else {
                self.visited_fragments
                    .insert(node.fragment_name.to_string(), -1);
            }

            let fragment = self.ctx.known_fragments.get(&node.fragment_name.as_str());
            if let Some(fragment) = fragment {
                let fragment_depth = self.count_depth(fragment.into(), Some(0));

                depth = cmp::max(depth, parent_depth + fragment_depth);
                if Some(&-1) == self.visited_fragments.get(&node.fragment_name) {
                    self.visited_fragments
                        .insert(node.fragment_name.to_string(), fragment_depth);
                }
            }
        }

        depth
    }
}

#[cfg(test)]
mod tests {
    use graphql_parser::parse_schema;
    use graphql_tools::validation::validate::ValidationPlan;
    use hive_router_config::query_complexity::MaxDepthRuleConfig;

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
            graphql_parser::parse_query(QUERY).expect("Failed to parse query");

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
            graphql_parser::parse_query(QUERY).expect("Failed to parse query");

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

        let doc: graphql_tools::static_graphql::query::Document = graphql_parser::parse_query(
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

        let doc: graphql_tools::static_graphql::query::Document = graphql_parser::parse_query(
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

        let doc: graphql_tools::static_graphql::query::Document = graphql_parser::parse_query(
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

    const INTROSPECTION_QUERY: &'static str = include_str!("introspection_query.graphql");
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

        let doc = graphql_parser::parse_query(INTROSPECTION_QUERY).expect("Failed to parse query");

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

        let doc: graphql_tools::static_graphql::query::Document = graphql_parser::parse_query(
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
            graphql_parser::parse_query(QUERY).expect("Failed to parse query");

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
            graphql_parser::parse_query(QUERY).expect("Failed to parse query");

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

        let doc: graphql_tools::static_graphql::query::Document = graphql_parser::parse_query(
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

        let doc: graphql_tools::static_graphql::query::Document = graphql_parser::parse_query(
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
