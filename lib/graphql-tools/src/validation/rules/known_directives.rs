use super::ValidationRule;
use crate::ast::{visit_document, OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::{
    Directive, Field, FragmentDefinition, InlineFragment, OperationDefinition,
};
use crate::static_graphql::schema::DirectiveLocation;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Known Directives
///
/// A GraphQL document is only valid if all `@directives` are known by the
/// schema and legally positioned.
///
/// See https://spec.graphql.org/draft/#sec-Directives-Are-Defined
pub struct KnownDirectives {
    recent_location: Option<DirectiveLocation>,
}

impl Default for KnownDirectives {
    fn default() -> Self {
        Self::new()
    }
}

impl KnownDirectives {
    pub fn new() -> Self {
        KnownDirectives {
            recent_location: None,
        }
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for KnownDirectives {
    fn enter_operation_definition(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        operation_definition: &crate::static_graphql::query::OperationDefinition,
    ) {
        self.recent_location = Some(match operation_definition {
            OperationDefinition::Mutation(_) => DirectiveLocation::Mutation,
            OperationDefinition::Query(_) => DirectiveLocation::Query,
            OperationDefinition::SelectionSet(_) => DirectiveLocation::Query,
            OperationDefinition::Subscription(_) => DirectiveLocation::Subscription,
        })
    }

    fn leave_operation_definition(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        _: &OperationDefinition,
    ) {
        self.recent_location = None;
    }

    fn enter_field(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        _: &Field,
    ) {
        self.recent_location = Some(DirectiveLocation::Field);
    }

    fn leave_field(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        _: &Field,
    ) {
        self.recent_location = None;
    }

    fn enter_fragment_definition(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        _: &FragmentDefinition,
    ) {
        self.recent_location = Some(DirectiveLocation::FragmentDefinition);
    }

    fn leave_fragment_definition(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        _: &FragmentDefinition,
    ) {
        self.recent_location = None;
    }

    fn enter_fragment_spread(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        _: &crate::static_graphql::query::FragmentSpread,
    ) {
        self.recent_location = Some(DirectiveLocation::FragmentSpread);
    }

    fn leave_fragment_spread(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        _: &crate::static_graphql::query::FragmentSpread,
    ) {
        self.recent_location = None;
    }

    fn enter_inline_fragment(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        _: &InlineFragment,
    ) {
        self.recent_location = Some(DirectiveLocation::InlineFragment);
    }

    fn leave_inline_fragment(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        _: &InlineFragment,
    ) {
        self.recent_location = None;
    }

    fn enter_directive(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        directive: &Directive,
    ) {
        if let Some(directive_type) = visitor_context.directives.get(&directive.name) {
            if let Some(current_location) = &self.recent_location {
                if !directive_type
                    .locations
                    .iter()
                    .any(|l| l == current_location)
                {
                    user_context.report_error(ValidationError {
                        error_code: self.error_code(),
                        locations: vec![directive.position],
                        message: format!(
                            "Directive \"@{}\" may not be used on {}",
                            directive.name,
                            current_location.as_str()
                        ),
                    });
                }
            }
        } else {
            user_context.report_error(ValidationError {
                error_code: self.error_code(),
                locations: vec![directive.position],
                message: format!("Unknown directive \"@{}\".", directive.name),
            });
        }
    }
}

impl ValidationRule for KnownDirectives {
    fn error_code<'a>(&self) -> &'a str {
        "KnownDirectives"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut KnownDirectives::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn no_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownDirectives::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          name
          ...Frag
        }
  
        fragment Frag on Dog {
          name
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn standard_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownDirectives::new()));
    let errors = test_operation_with_schema(
        "{
          human @skip(if: false) {
            name
            pets {
              ... on Dog @include(if: true) {
                name
              }
            }
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn unknown_directive() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownDirectives::new()));
    let errors = test_operation_with_schema(
        "{
          human @unknown(directive: \"value\") {
            name
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 1);
}

#[test]
fn many_unknown_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownDirectives::new()));
    let errors = test_operation_with_schema(
        "{
          __typename @unknown
          human @unknown {
            name
            pets @unknown {
              name
            }
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 3);
}

#[test]
fn well_placed_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownDirectives::new()));
    let errors = test_operation_with_schema(
        "
        # TODO: update once this is released https://github.com/graphql-rust/graphql-parser/issues/60 query ($var: Boolean @onVariableDefinition)
        query ($var: Boolean) @onQuery {
          human @onField {
            ...Frag @onFragmentSpread
            ... @onInlineFragment {
              name @onField
            }
          }
        }
  
        mutation @onMutation {
          someField @onField
        }
  
        subscription @onSubscription {
          someField @onField
        }
  
        fragment Frag on Human @onFragmentDefinition {
          name @onField
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn misplaced_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownDirectives::new()));
    let errors = test_operation_with_schema(
        "  query ($var: Boolean) @onMutation {
      human @onQuery {
        ...Frag @onQuery
        ... @onQuery {
          name @onQuery
        }
      }
    }

    mutation @onQuery {
      someField @onQuery
    }

    subscription @onQuery {
      someField @onQuery
    }

    fragment Frag on Human @onQuery {
      name @onQuery
    }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 11);
}
