use std::{cmp, collections::HashMap};

use graphql_tools::{
    ast::{OperationVisitor, OperationVisitorContext},
    static_graphql::query::{Definition, Document, FragmentDefinition, Selection},
    validation::{
        rules::{ValidationRule, ValidationVisitor},
        utils::{ValidationError, ValidationErrorContext},
    },
};
use hive_router_config::limits::MaxDepthRuleConfig;

use crate::pipeline::validation::shared::{CountableNode, VisitedFragment};

pub struct MaxDepthRule {
    pub config: MaxDepthRuleConfig,
}

impl ValidationRule for MaxDepthRule {
    fn error_code(&self) -> &'static str {
        "MAX_DEPTH_EXCEEDED"
    }

    fn visitor<'doc>(&self) -> ValidationVisitor<'doc> {
        Box::new(MaxDepthVisitor {
            config: self.config.clone(),
            visited_fragments: HashMap::new(),
        })
    }
}

struct MaxDepthVisitor<'a> {
    config: MaxDepthRuleConfig,
    visited_fragments: HashMap<&'a str, VisitedFragment>,
}

impl<'a> MaxDepthVisitor<'a> {
    fn check_limit(&self, count: usize) -> Result<usize, ValidationError> {
        if count > self.config.n {
            Err(ValidationError {
                locations: vec![],
                message: "Query depth limit exceeded.".to_string(),
                error_code: "MAX_DEPTH_EXCEEDED",
            })
        } else {
            Ok(count)
        }
    }

    fn count_depth(
        &mut self,
        known_fragments: &HashMap<&'a str, &'a FragmentDefinition>,
        node: CountableNode<'a>,
        parent_depth: Option<usize>,
    ) -> Result<usize, ValidationError> {
        // If introspection queries are to be ignored, skip them from the root
        if self.config.ignore_introspection {
            if let CountableNode::Field(field) = node {
                let field_name = field.name.as_str();
                if field_name == "__schema" || field_name == "__type" {
                    return Ok(0);
                }
            }
        }

        // Initialize parent depth
        let mut parent_depth = parent_depth.unwrap_or(0);

        // Current depth starts as parent depth
        let mut depth = self.check_limit(parent_depth)?;

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
                    self.count_depth(
                        known_fragments,
                        child.into(),
                        Some(parent_depth + increase_by),
                    )?,
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
            match self.visited_fragments.get(fragment_name) {
                Some(VisitedFragment::Counted(visited_fragment_depth)) => {
                    // If it was already visited, return the cached depth
                    return self.check_limit(parent_depth + visited_fragment_depth);
                }
                Some(VisitedFragment::Visiting) => return Ok(depth),
                None => {}
            }

            // If not, mark it as Visiting initially to avoid infinite loops,
            // because fragments can refer itself recursively at some point.
            // See the tests at the bottom of this file to understand the use cases fully.
            self.visited_fragments
                .insert(fragment_name, VisitedFragment::Visiting);

            // Look up the fragment definition by its name
            if let Some(fragment) = known_fragments.get(fragment_name) {
                // Count the depth of the fragment
                let fragment_depth = self.count_depth(known_fragments, fragment.into(), Some(0))?;

                // Update it with the actual depth.
                self.visited_fragments
                    .insert(fragment_name, VisitedFragment::Counted(fragment_depth));

                let parent_plus_fragment = self.check_limit(parent_depth + fragment_depth)?;

                // Update the overall depth
                depth = cmp::max(depth, parent_plus_fragment);
            }
        }

        Ok(depth)
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for MaxDepthVisitor<'a> {
    fn enter_document(
        &mut self,
        context: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        document: &'a Document,
    ) {
        self.visited_fragments = HashMap::with_capacity(context.known_fragments.len());

        for definition in &document.definitions {
            let Definition::Operation(op) = definition else {
                continue;
            };
            if let Err(err) = self.count_depth(&context.known_fragments, op.into(), None) {
                user_context.report_error(err);
            }
        }
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
            config: MaxDepthRuleConfig {
                n: 3,
                ignore_introspection: true,
                flatten_fragments: false,
            },
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
                ignore_introspection: true,
                flatten_fragments: false,
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
    fn rejects_fragment_exceeding_max_depth() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 4,
                ignore_introspection: true,
                flatten_fragments: false,
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
        assert_eq!(error.message, "Query depth limit exceeded.");
    }

    #[test]
    fn rejects_flattened_fragment_exceeding_max_depth() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 2,
                ignore_introspection: true,
                flatten_fragments: true,
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
        assert_eq!(error.message, "Query depth limit exceeded.");
    }

    #[test]
    fn rejects_flattened_inline_fragment_exceeding_max_depth() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 2,
                ignore_introspection: true,
                flatten_fragments: true,
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
        assert_eq!(error.message, "Query depth limit exceeded.");
    }

    const INTROSPECTION_QUERY: &'static str =
        include_str!("test_fixtures/introspection_query.fixture.graphql");
    #[test]
    fn allows_introspection_queries_when_ignored() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 2,
                ignore_introspection: true,
                flatten_fragments: false,
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
                ignore_introspection: true,
                flatten_fragments: false,
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
        assert_eq!(error.message, "Query depth limit exceeded.");
    }

    #[test]
    fn rejects_with_a_generic_message_when_expose_limits_is_false() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 2,
                ignore_introspection: true,
                flatten_fragments: false,
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
    fn rejects_for_fragment_named_schema_exceeding_max_depth() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 6,
                ignore_introspection: true,
                flatten_fragments: false,
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
        assert_eq!(error.message, "Query depth limit exceeded.");
    }

    #[test]
    fn rejects_for_exceeding_max_depth_by_reusing_a_cached_fragment() {
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 6,
                ignore_introspection: true,
                flatten_fragments: false,
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
        assert_eq!(error.message, "Query depth limit exceeded.");
    }

    #[test]
    fn skips_unknown_fragment() {
        // This rule is not responsible for checking unknown fragments.
        // That should be done by another rule.
        // Here we just ensure that unknown fragments are skipped
        let validation_plan = ValidationPlan::from(vec![Box::new(MaxDepthRule {
            config: MaxDepthRuleConfig {
                n: 2,
                ignore_introspection: true,
                flatten_fragments: false,
            },
        })]);

        let schema: graphql_tools::static_graphql::schema::Document =
            parse_schema(TYPE_DEFS).expect("Failed to parse schema");

        let doc: graphql_tools::static_graphql::query::Document = parse_query(
            r#"
      query {
        ...UnknownFragment
      }
        "#,
        )
        .expect("Failed to parse query");

        let errors = graphql_tools::validation::validate::validate(&schema, &doc, &validation_plan);

        assert!(
            errors.is_empty(),
            "Expected no validation errors but found some"
        );
    }
}
