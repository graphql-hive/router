use crate::ast::ext::TypeDefinitionExtension;
use crate::ast::{visit_document, FieldByNameExtension, OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::{Field, OperationDefinition, Selection};
use crate::validation::utils::{ValidationError, ValidationErrorContext};

use super::ValidationRule;

/// Fields on correct type
///
/// A GraphQL document is only valid if all fields selected are defined by the
/// parent type, or are an allowed meta field such as __typename.
///
/// See https://spec.graphql.org/draft/#sec-Field-Selections
pub struct FieldsOnCorrectType;

impl Default for FieldsOnCorrectType {
    fn default() -> Self {
        Self::new()
    }
}

impl FieldsOnCorrectType {
    pub fn new() -> Self {
        FieldsOnCorrectType
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for FieldsOnCorrectType {
    fn enter_operation_definition(
        &mut self,
        _: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        operation: &OperationDefinition,
    ) {
        // https://spec.graphql.org/October2021/#note-bc213
        if let OperationDefinition::Subscription(subscription) = operation {
            for selection in &subscription.selection_set.items {
                if let Selection::Field(field) = selection {
                    if field.name == "__typename" {
                        user_context.report_error(ValidationError {
                          error_code: self.error_code(),
                          message: "`__typename` may not be included as a root field in a subscription operation".to_string(),
                          locations: vec![subscription.position],
                        });
                    }
                }
            }
        }
    }

    fn enter_field(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        field: &Field,
    ) {
        if let Some(parent_type) = visitor_context.current_parent_type() {
            let field_name = &field.name;
            let type_name = parent_type.name();

            if field.name.starts_with("__") {
                return;
            }

            if parent_type.field_by_name(field_name).is_none() {
                user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    locations: vec![field.position],
                    message: format!(
                        "Cannot query field \"{}\" on type \"{}\".",
                        field_name, type_name
                    ),
                });
            }
        }
    }
}

impl ValidationRule for FieldsOnCorrectType {
    fn error_code<'a>(&self) -> &'a str {
        "FieldsOnCorrectType"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut FieldsOnCorrectType::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[cfg(test)]
pub static FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA: &str = "
  interface Pet {
    name: String
  }
  type Dog implements Pet {
    name: String
    nickname: String
    barkVolume: Int
  }
  type Cat implements Pet {
    name: String
    nickname: String
    meowVolume: Int
  }
  union CatOrDog = Cat | Dog
  type Human {
    name: String
    pets: [Pet]
  }
  type Query {
    human: Human
  }
  type Mutation {
    deletePetByName(name: String): Pet
  }
  type Subscription {
    onNewPet: Pet
  }
";

#[test]
fn object_field_selection() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment objectFieldSelection on Dog {
          __typename
          name
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn aliased_object_field_selection() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment aliasedObjectFieldSelection on Dog {
          tn : __typename
          otherName : name
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn interface_field_selection() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment interfaceFieldSelection on Pet {
          __typename
          name
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn aliased_interface_field_selection() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment interfaceFieldSelection on Pet {
          otherName : name
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn lying_alias_selection() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment lyingAliasSelection on Dog {
          name : nickname
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn ignores_fields_on_unknown_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment unknownSelection on UnknownType {
          unknownField
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn unknown_query_field() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "query test {
          unknownField
        }",
        &FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"unknownField\" on type \"Query\"."]
    );
}

#[test]
fn unknown_mutation_field() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "mutation test {
          unknownField
        }",
        &FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"unknownField\" on type \"Mutation\"."]
    );
}

#[test]
fn unknown_subscription_field() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "subscription test {
          unknownField
        }",
        &FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"unknownField\" on type \"Subscription\"."]
    );
}

#[test]
fn reports_errors_when_type_is_known_again() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment typeKnownAgain on Pet {
          unknown_pet_field {
            ... on Cat {
              unknown_cat_field
            }
          }
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages,
        vec![
            "Cannot query field \"unknown_pet_field\" on type \"Pet\".",
            "Cannot query field \"unknown_cat_field\" on type \"Cat\"."
        ]
    );
}

#[test]
fn field_not_defined_on_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment fieldNotDefined on Dog {
          meowVolume
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"meowVolume\" on type \"Dog\"."]
    );
}

#[test]
fn ignores_deeply_unknown_field() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment deepFieldNotDefined on Dog {
          unknown_field {
            deeper_unknown_field
          }
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"unknown_field\" on type \"Dog\"."]
    );
}

#[test]
fn sub_field_not_defined() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment subFieldNotDefined on Human {
          pets {
            unknown_field
          }
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"unknown_field\" on type \"Pet\"."]
    );
}

#[test]
fn field_not_defined_on_inline_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment fieldNotDefined on Pet {
          ... on Dog {
            meowVolume
          }
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"meowVolume\" on type \"Dog\"."]
    );
}

#[test]
fn aliased_field_target_not_defined() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment aliasedFieldTargetNotDefined on Dog {
          volume : mooVolume
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"mooVolume\" on type \"Dog\"."]
    );
}

#[test]
fn aliased_lying_field_target_not_defined() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment aliasedLyingFieldTargetNotDefined on Dog {
          barkVolume : kawVolume
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"kawVolume\" on type \"Dog\"."]
    );
}

#[test]
fn not_defined_on_interface() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment notDefinedOnInterface on Pet {
          tailLength
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"tailLength\" on type \"Pet\"."]
    );
}

#[test]
fn defined_on_implementors_but_not_on_interface() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment definedOnImplementorsButNotInterface on Pet {
          nickname
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"nickname\" on type \"Pet\"."]
    );
}

#[test]
fn direct_field_selection_on_union() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment directFieldSelectionOnUnion on CatOrDog {
          directField
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"directField\" on type \"CatOrDog\"."]
    );
}

#[test]
fn defined_on_implementors_queried_on_union() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment definedOnImplementorsQueriedOnUnion on CatOrDog {
          name
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Cannot query field \"name\" on type \"CatOrDog\"."]
    );
}

#[test]
fn meta_field_selection_on_union() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment directFieldSelectionOnUnion on CatOrDog {
          __typename
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn valid_field_in_inline_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "fragment objectFieldSelection on Pet {
          ... on Dog {
            name
          }
          ... {
            name
          }
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn forbidden_typename_on_subscription_type() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(FieldsOnCorrectType {}));
    let errors = test_operation_with_schema(
        "subscription {
          __typename 
        }",
        FIELDS_ON_CORRECT_TYPE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["`__typename` may not be included as a root field in a subscription operation"]
    );
}
