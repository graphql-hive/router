use super::ValidationRule;
use crate::ast::{visit_document, OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::*;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// No unused fragments
///
/// A GraphQL document is only valid if all fragment definitions are spread
/// within operations, or spread within other fragments spread within operations.
///
/// See https://spec.graphql.org/draft/#sec-Fragments-Must-Be-Used
pub struct NoUnusedFragments<'a> {
    fragments_in_use: Vec<&'a str>,
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for NoUnusedFragments<'a> {
    fn enter_fragment_spread(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        fragment_spread: &'a FragmentSpread,
    ) {
        self.fragments_in_use
            .push(fragment_spread.fragment_name.as_str());
    }

    fn leave_document(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        _document: &Document,
    ) {
        visitor_context
            .known_fragments
            .iter()
            .filter_map(|(fragment_name, _fragment)| {
                if !self.fragments_in_use.contains(fragment_name) {
                    Some(fragment_name)
                } else {
                    None
                }
            })
            .for_each(|unused_fragment_name| {
                user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    locations: vec![],
                    message: format!("Fragment \"{}\" is never used.", unused_fragment_name),
                });
            });
    }
}

impl<'a> Default for NoUnusedFragments<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> NoUnusedFragments<'a> {
    pub fn new() -> Self {
        NoUnusedFragments {
            fragments_in_use: Vec::new(),
        }
    }
}

impl<'n> ValidationRule for NoUnusedFragments<'n> {
    fn error_code<'a>(&self) -> &'a str {
        "NoUnusedFragments"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut NoUnusedFragments::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn all_fragment_names_are_used() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedFragments::new()));
    let errors = test_operation_with_schema(
        "{
          human(id: 4) {
            ...HumanFields1
            ... on Human {
              ...HumanFields2
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
fn all_fragment_names_are_used_by_multiple_operations() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedFragments::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          human(id: 4) {
            ...HumanFields1
          }
        }
        query Bar {
          human(id: 4) {
            ...HumanFields2
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
        }
  ",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn contains_unknown_fragments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedFragments::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          human(id: 4) {
            ...HumanFields1
          }
        }
        query Bar {
          human(id: 4) {
            ...HumanFields2
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
        }
        fragment Unused1 on Human {
          name
        }
        fragment Unused2 on Human {
          name
        }
  ",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
}

// TODO: Fix this one :( It's not working
#[test]
#[ignore = "Fix this one :( It's not working"]
fn contains_unknown_fragments_with_ref_cycle() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedFragments::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          human(id: 4) {
            ...HumanFields1
          }
        }
        query Bar {
          human(id: 4) {
            ...HumanFields2
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
        }
        fragment Unused1 on Human {
          name
          ...Unused2
        }
        fragment Unused2 on Human {
          name
          ...Unused1
        }
  ",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec![
            "Fragment \"Unused1\" is never used.",
            "Fragment \"Unused2\" is never used."
        ]
    );
}

#[test]
fn contains_unknown_and_undef_fragments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(NoUnusedFragments::new()));
    let errors = test_operation_with_schema(
        "query Foo {
          human(id: 4) {
            ...bar
          }
        }
        fragment foo on Human {
          name
        }
  ",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Fragment \"foo\" is never used.",]);
}
