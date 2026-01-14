use super::ValidationRule;
use crate::ast::{visit_document, OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::*;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Known fragment names
///
/// A GraphQL document is only valid if all `...Fragment` fragment spreads refer
/// to fragments defined in the same document.
///
/// See https://spec.graphql.org/draft/#sec-Fragment-spread-target-defined
pub struct KnownFragmentNames;

impl Default for KnownFragmentNames {
    fn default() -> Self {
        Self::new()
    }
}

impl KnownFragmentNames {
    pub fn new() -> Self {
        KnownFragmentNames
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for KnownFragmentNames {
    fn enter_fragment_spread(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        fragment_spread: &FragmentSpread,
    ) {
        if !visitor_context
            .known_fragments
            .contains_key(fragment_spread.fragment_name.as_str())
        {
            user_context.report_error(ValidationError {
                error_code: self.error_code(),
                locations: vec![fragment_spread.position],
                message: format!("Unknown fragment \"{}\".", fragment_spread.fragment_name),
            })
        }
    }
}

impl ValidationRule for KnownFragmentNames {
    fn error_code<'a>(&self) -> &'a str {
        "KnownFragmentNames"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut KnownFragmentNames::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn valid_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownFragmentNames {}));
    let errors = test_operation_with_schema(
        "{
          human(id: 4) {
            ...HumanFields1
            ... on Human {
              ...HumanFields2
            }
            ... {
              name
            }
          }
        }
        fragment HumanFields1 on Human {
          name
          ...HumanFields3
        }
        fragment HumanFields2 on Human {
          name
        }
        fragment HumanFields3 on Human {
          name
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn invalid_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(KnownFragmentNames {}));
    let errors = test_operation_with_schema(
        "{
          human(id: 4) {
            ...UnknownFragment1
            ... on Human {
              ...UnknownFragment2
            }
          }
        }
        fragment HumanFields on Human {
          name
          ...UnknownFragment3
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages,
        vec![
            "Unknown fragment \"UnknownFragment1\".",
            "Unknown fragment \"UnknownFragment2\".",
            "Unknown fragment \"UnknownFragment3\".",
        ]
    );
}
