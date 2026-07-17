use std::collections::{HashMap, HashSet, VecDeque};

use super::ValidationRule;
use crate::ast::{OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::*;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// No unused fragments
///
/// A GraphQL document is only valid if all fragment definitions are spread
/// within operations, or spread within other fragments spread within operations.
///
/// See https://spec.graphql.org/draft/#sec-Fragments-Must-Be-Used
pub struct NoUnusedFragments<'doc> {
    fragments_in_use: Vec<&'doc str>,
    current_fragment_spreads: Vec<&'doc str>,
    current_fragment: Option<&'doc str>,
    fragment_spreads: HashMap<&'doc str, Vec<&'doc str>>,
}

impl<'doc> OperationVisitor<'doc, ValidationErrorContext> for NoUnusedFragments<'doc> {
    fn enter_fragment_definition(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        fragment: &'doc FragmentDefinition,
    ) {
        self.current_fragment = Some(fragment.name.as_str());
        self.current_fragment_spreads = Vec::new();
    }

    fn leave_fragment_definition(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        _: &FragmentDefinition,
    ) {
        if let Some(name) = self.current_fragment.take() {
            self.fragment_spreads
                .insert(name, std::mem::take(&mut self.current_fragment_spreads));
        }
    }

    fn enter_fragment_spread(
        &mut self,
        _: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        fragment_spread: &'doc FragmentSpread,
    ) {
        let name = fragment_spread.fragment_name.as_str();
        if self.current_fragment.is_some() {
            self.current_fragment_spreads.push(name);
        } else {
            self.fragments_in_use.push(name);
        }
    }

    fn leave_document(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        _document: &Document,
    ) {
        let mut reachable: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = self.fragments_in_use.iter().copied().collect();

        while let Some(frag) = queue.pop_front() {
            if !reachable.insert(frag) {
                continue;
            }

            let Some(spreads) = self.fragment_spreads.get(frag) else {
                continue;
            };

            for spread in spreads {
                if !reachable.contains(spread) {
                    queue.push_back(spread);
                }
            }
        }

        visitor_context
            .known_fragments
            .keys()
            .filter(|fragment_name| !reachable.contains(*fragment_name))
            .for_each(|unused_fragment_name| {
                user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    locations: vec![],
                    message: format!("Fragment \"{}\" is never used.", unused_fragment_name),
                });
            });
    }
}

impl Default for NoUnusedFragments<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl NoUnusedFragments<'_> {
    pub fn new() -> Self {
        NoUnusedFragments {
            fragments_in_use: Vec::new(),
            current_fragment_spreads: Vec::new(),
            current_fragment: None,
            fragment_spreads: HashMap::new(),
        }
    }
}

impl ValidationRule for NoUnusedFragments<'_> {
    fn error_code(&self) -> &'static str {
        "NoUnusedFragments"
    }

    fn visitor<'doc>(&self) -> super::ValidationVisitor<'doc> {
        Box::new(NoUnusedFragments::new())
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

#[test]
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
    assert!(messages.contains(&&"Fragment \"Unused1\" is never used.".to_owned()));
    assert!(messages.contains(&&"Fragment \"Unused2\" is never used.".to_owned()));
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
