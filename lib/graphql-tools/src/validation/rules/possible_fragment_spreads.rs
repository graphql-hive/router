use super::ValidationRule;
use crate::ast::ext::TypeDefinitionExtension;
use crate::ast::{
    visit_document, ImplementingInterfaceExtension, OperationVisitor, OperationVisitorContext,
    PossibleTypesExtension, SchemaDocumentExtension,
};
use crate::static_graphql::query::TypeCondition;
use crate::static_graphql::schema;
use crate::validation::utils::{ValidationError, ValidationErrorContext};

/// Possible fragment spread
///
/// A fragment spread is only valid if the type condition could ever possibly
/// be true: if there is a non-empty intersection of the possible parent types,
/// and possible types which pass the type condition.
///
/// https://spec.graphql.org/draft/#sec-Fragment-spread-is-possible
pub struct PossibleFragmentSpreads;

impl Default for PossibleFragmentSpreads {
    fn default() -> Self {
        Self::new()
    }
}

impl PossibleFragmentSpreads {
    pub fn new() -> Self {
        Self {}
    }
}

/**
 * Provided two composite types, determine if they "overlap". Two composite
 * types overlap when the Sets of possible concrete types for each intersect.
 *
 * This is often used to determine if a fragment of a given type could possibly
 * be visited in a context of another type.
 *
 * This function is commutative.
 */
pub fn do_types_overlap(
    schema: &schema::Document,
    t1: &schema::TypeDefinition,
    t2: &schema::TypeDefinition,
) -> bool {
    if t1.name().eq(t2.name()) {
        return true;
    }

    if t1.is_abstract_type() {
        if t2.is_abstract_type() {
            let possible_types = t1.possible_types(schema);

            return possible_types
                .into_iter()
                .filter(|possible_type| t2.has_concrete_sub_type(possible_type))
                .count()
                > 0;
        }

        return t1.has_sub_type(t2);
    }

    if t2.is_abstract_type() {
        return t2.has_sub_type(t1);
    }

    false
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for PossibleFragmentSpreads {
    fn enter_inline_fragment(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        _inline_fragment: &crate::static_graphql::query::InlineFragment,
    ) {
        if let Some(frag_schema_type) = visitor_context.current_type() {
            if let Some(parent_type) = visitor_context.current_parent_type() {
                if frag_schema_type.is_composite_type()
                    && parent_type.is_composite_type()
                    && !do_types_overlap(visitor_context.schema, frag_schema_type, parent_type)
                {
                    user_context.report_error(ValidationError {error_code: self.error_code(),
                      locations: vec![],
                      message: format!("Fragment cannot be spread here as objects of type \"{}\" can never be of type \"{}\".", parent_type.name(), frag_schema_type.name()),
                    })
                }
            }
        }
    }

    fn enter_fragment_spread(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        fragment_spread: &crate::static_graphql::query::FragmentSpread,
    ) {
        if let Some(actual_fragment) = visitor_context
            .known_fragments
            .get(fragment_spread.fragment_name.as_str())
        {
            let TypeCondition::On(fragment_type_name) = &actual_fragment.type_condition;

            if let Some(fragment_type) = visitor_context.schema.type_by_name(fragment_type_name) {
                if let Some(parent_type) = visitor_context.current_parent_type() {
                    if fragment_type.is_composite_type()
                        && parent_type.is_composite_type()
                        && !do_types_overlap(visitor_context.schema, fragment_type, parent_type)
                    {
                        user_context.report_error(ValidationError {error_code: self.error_code(),
                        locations: vec![],
                        message: format!("Fragment \"{}\" cannot be spread here as objects of type \"{}\" can never be of type \"{}\".", actual_fragment.name, parent_type.name(), fragment_type_name),
                      })
                    }
                }
            }
        }
    }
}

impl ValidationRule for PossibleFragmentSpreads {
    fn error_code<'a>(&self) -> &'a str {
        "PossibleFragmentSpreads"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut PossibleFragmentSpreads::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[cfg(test)]
static RULE_TEST_SCHEMA: &str = "
  interface Being {
    name: String
  }
  interface Pet implements Being {
    name: String
  }
  type Dog implements Being & Pet {
    name: String
    barkVolume: Int
  }
  type Cat implements Being & Pet {
    name: String
    meowVolume: Int
  }
  union CatOrDog = Cat | Dog
  interface Intelligent {
    iq: Int
  }
  type Human implements Being & Intelligent {
    name: String
    pets: [Pet]
    iq: Int
  }
  type Alien implements Being & Intelligent {
    name: String
    iq: Int
  }
  union DogOrHuman = Dog | Human
  union HumanOrAlien = Human | Alien
  type Query {
    catOrDog: CatOrDog
    dogOrHuman: DogOrHuman
    humanOrAlien: HumanOrAlien
  }
";

#[test]
fn of_the_same_object() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment objectWithinObject on Dog { ...dogFragment }
        fragment dogFragment on Dog { barkVolume }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn of_the_same_object_with_inline_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment objectWithinObjectAnon on Dog { ... on Dog { barkVolume } }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn object_into_an_implemented_interface() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment objectWithinInterface on Pet { ...dogFragment }
        fragment dogFragment on Dog { barkVolume }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn object_into_containing_union() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment objectWithinUnion on CatOrDog { ...dogFragment }
        fragment dogFragment on Dog { barkVolume }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn union_into_contained_object() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment unionWithinObject on Dog { ...catOrDogFragment }
        fragment catOrDogFragment on CatOrDog { __typename }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn union_into_overlapping_interface() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment unionWithinInterface on Pet { ...catOrDogFragment }
        fragment catOrDogFragment on CatOrDog { __typename }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn union_into_overlapping_union() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment unionWithinUnion on DogOrHuman { ...catOrDogFragment }
        fragment catOrDogFragment on CatOrDog { __typename }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn interface_into_implemented_object() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment interfaceWithinObject on Dog { ...petFragment }
        fragment petFragment on Pet { name }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn interface_into_overlapping_interface() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment interfaceWithinInterface on Pet { ...beingFragment }
        fragment beingFragment on Being { name }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn interface_into_overlapping_interface_in_inline_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment interfaceWithinInterface on Pet { ... on Being { name } }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn interface_into_overlapping_union() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment interfaceWithinUnion on CatOrDog { ...petFragment }
        fragment petFragment on Pet { name }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

// caught by FragmentsOnCompositeTypesRule
#[test]
fn ignores_incorrect_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment petFragment on Pet { ...badInADifferentWay }
        fragment badInADifferentWay on String { name }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

// caught by KnownFragmentNamesRule
#[test]
fn ignores_unknown_fragments() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment petFragment on Pet { ...UnknownFragment }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn different_object_into_object() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment invalidObjectWithinObject on Cat { ...dogFragment }
        fragment dogFragment on Dog { barkVolume }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fragment \"dogFragment\" cannot be spread here as objects of type \"Cat\" can never be of type \"Dog\"."
    ])
}

#[test]
fn different_object_into_object_in_inline_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment invalidObjectWithinObjectAnon on Cat {
          ... on Dog { barkVolume }
        }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
      "Fragment cannot be spread here as objects of type \"Cat\" can never be of type \"Dog\"."
    ]
    )
}

#[test]
fn object_into_not_implementing_interface() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment invalidObjectWithinInterface on Pet { ...humanFragment }
        fragment humanFragment on Human { pets { name } }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
          "Fragment \"humanFragment\" cannot be spread here as objects of type \"Pet\" can never be of type \"Human\"."
        ]
    )
}

#[test]
fn object_into_not_containing_union() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment invalidObjectWithinUnion on CatOrDog { ...humanFragment }
        fragment humanFragment on Human { pets { name } }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
          "Fragment \"humanFragment\" cannot be spread here as objects of type \"CatOrDog\" can never be of type \"Human\"."
        ]
    )
}

#[test]
fn union_into_not_contained_object() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment invalidUnionWithinObject on Human { ...catOrDogFragment }
        fragment catOrDogFragment on CatOrDog { __typename }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
          "Fragment \"catOrDogFragment\" cannot be spread here as objects of type \"Human\" can never be of type \"CatOrDog\"."
        ]
    )
}

#[test]
fn union_into_non_overlapping_interface() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment invalidUnionWithinInterface on Pet { ...humanOrAlienFragment }
        fragment humanOrAlienFragment on HumanOrAlien { __typename }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
          "Fragment \"humanOrAlienFragment\" cannot be spread here as objects of type \"Pet\" can never be of type \"HumanOrAlien\"."
        ]
    )
}

#[test]
fn union_into_non_overlapping_union() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment invalidUnionWithinUnion on CatOrDog { ...humanOrAlienFragment }
        fragment humanOrAlienFragment on HumanOrAlien { __typename }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
          "Fragment \"humanOrAlienFragment\" cannot be spread here as objects of type \"CatOrDog\" can never be of type \"HumanOrAlien\"."
        ]
    )
}

#[test]
fn interface_into_non_implementing_object() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment invalidInterfaceWithinObject on Cat { ...intelligentFragment }
        fragment intelligentFragment on Intelligent { iq }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
          "Fragment \"intelligentFragment\" cannot be spread here as objects of type \"Cat\" can never be of type \"Intelligent\"."
        ]
    )
}

#[test]
fn interface_into_non_overlapping_interface() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment invalidInterfaceWithinInterface on Pet {
          ...intelligentFragment
        }
        fragment intelligentFragment on Intelligent { iq }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
          "Fragment \"intelligentFragment\" cannot be spread here as objects of type \"Pet\" can never be of type \"Intelligent\"."
        ]
    )
}

#[test]
fn interface_into_non_overlapping_interface_in_inline_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(PossibleFragmentSpreads {}));
    let errors = test_operation_with_schema(
        "fragment invalidInterfaceWithinInterfaceAnon on Pet {
          ...on Intelligent { iq }
        }",
        RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
          "Fragment cannot be spread here as objects of type \"Pet\" can never be of type \"Intelligent\"."
        ]
    )
}
