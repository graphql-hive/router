use std::collections::{HashMap, HashSet};

use crate::{
    ast::{
        visit_document, AstNodeWithName, OperationVisitor, OperationVisitorContext,
        SchemaDocumentExtension,
    },
    static_graphql::query::{Type, Value, VariableDefinition},
    validation::utils::{ValidationError, ValidationErrorContext},
};

use super::ValidationRule;

/// Variables in allowed position
///
/// Variable usages must be compatible with the arguments they are passed to.
///
/// See https://spec.graphql.org/draft/#sec-All-Variable-Usages-are-Allowed
#[derive(Default)]
pub struct VariablesInAllowedPosition<'a> {
    spreads: HashMap<Scope<'a>, HashSet<&'a str>>,
    variable_usages: HashMap<Scope<'a>, Vec<(&'a str, &'a Type)>>,
    variable_defs: HashMap<Scope<'a>, Vec<&'a VariableDefinition>>,
    current_scope: Option<Scope<'a>>,
}

impl<'a> VariablesInAllowedPosition<'a> {
    pub fn new() -> Self {
        VariablesInAllowedPosition {
            spreads: HashMap::new(),
            variable_usages: HashMap::new(),
            variable_defs: HashMap::new(),
            current_scope: None,
        }
    }

    fn collect_incorrect_usages(
        &self,
        from: &Scope<'a>,
        var_defs: &Vec<&VariableDefinition>,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        visited: &mut HashSet<Scope<'a>>,
    ) {
        if visited.contains(from) {
            return;
        }

        visited.insert(from.clone());

        if let Some(usages) = self.variable_usages.get(from) {
            for (var_name, var_type) in usages {
                if let Some(var_def) = var_defs.iter().find(|var_def| var_def.name == *var_name) {
                    let expected_type = match (&var_def.default_value, &var_def.var_type) {
                        (Some(_), Type::ListType(inner)) => Type::NonNullType(inner.clone()),
                        (Some(default_value), Type::NamedType(_)) => {
                            if let Value::Null = default_value {
                                var_def.var_type.clone()
                            } else {
                                Type::NonNullType(Box::new(var_def.var_type.clone()))
                            }
                        }
                        (_, t) => t.clone(),
                    };

                    if !visitor_context.schema.is_subtype(&expected_type, var_type) {
                        user_context.report_error(ValidationError {
                          error_code: self.error_code(),
                            message: format!("Variable \"${}\" of type \"{}\" used in position expecting type \"{}\".",
                                var_name,
                                expected_type,
                                var_type,
                            ),
                            locations: vec![var_def.position],
                        });
                    }
                }
            }
        }

        if let Some(spreads) = self.spreads.get(from) {
            for spread in spreads {
                self.collect_incorrect_usages(
                    &Scope::Fragment(spread),
                    var_defs,
                    visitor_context,
                    user_context,
                    visited,
                );
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope<'a> {
    Operation(Option<&'a str>),
    Fragment(&'a str),
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for VariablesInAllowedPosition<'a> {
    fn leave_document(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        _: &crate::static_graphql::query::Document,
    ) {
        for (op_scope, var_defs) in &self.variable_defs {
            self.collect_incorrect_usages(
                op_scope,
                var_defs,
                visitor_context,
                user_context,
                &mut HashSet::new(),
            );
        }
    }

    fn enter_fragment_definition(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        fragment_definition: &'a crate::static_graphql::query::FragmentDefinition,
    ) {
        self.current_scope = Some(Scope::Fragment(&fragment_definition.name));
    }

    fn enter_operation_definition(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        operation_definition: &'a crate::static_graphql::query::OperationDefinition,
    ) {
        self.current_scope = Some(Scope::Operation(operation_definition.node_name()));
    }

    fn enter_fragment_spread(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        fragment_spread: &'a crate::static_graphql::query::FragmentSpread,
    ) {
        if let Some(scope) = &self.current_scope {
            self.spreads
                .entry(scope.clone())
                .or_default()
                .insert(&fragment_spread.fragment_name);
        }
    }

    fn enter_variable_definition(
        &mut self,
        _: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        variable_definition: &'a VariableDefinition,
    ) {
        if let Some(ref scope) = self.current_scope {
            self.variable_defs
                .entry(scope.clone())
                .or_default()
                .push(variable_definition);
        }
    }

    fn enter_variable_value(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        _: &mut ValidationErrorContext,
        variable_name: &'a str,
    ) {
        if let (Some(scope), Some(input_type)) = (
            &self.current_scope,
            visitor_context.current_input_type_literal(),
        ) {
            self.variable_usages
                .entry(scope.clone())
                .or_default()
                .push((variable_name, input_type));
        }
    }
}

impl<'v> ValidationRule for VariablesInAllowedPosition<'v> {
    fn error_code<'a>(&self) -> &'a str {
        "VariablesInAllowedPosition"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut VariablesInAllowedPosition::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn boolean_to_boolean() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($booleanArg: Boolean)
        {
          complicatedArgs {
            booleanArgField(booleanArg: $booleanArg)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn boolean_to_boolean_within_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "fragment booleanArgFrag on ComplicatedArgs {
          booleanArgField(booleanArg: $booleanArg)
        }
        query Query($booleanArg: Boolean)
        {
          complicatedArgs {
            ...booleanArgFrag
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);

    let errors = test_operation_with_schema(
        "query Query($booleanArg: Boolean)
      {
        complicatedArgs {
          ...booleanArgFrag
        }
      }
      fragment booleanArgFrag on ComplicatedArgs {
        booleanArgField(booleanArg: $booleanArg)
      }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn boolean_nonnull_to_boolean() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($nonNullBooleanArg: Boolean!)
        {
          complicatedArgs {
            booleanArgField(booleanArg: $nonNullBooleanArg)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn string_list_to_string_list() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($stringListVar: [String])
        {
          complicatedArgs {
            stringListArgField(stringListArg: $stringListVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn string_list_nonnull_to_string_list() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($stringListVar: [String!])
        {
          complicatedArgs {
            stringListArgField(stringListArg: $stringListVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn string_to_string_list_in_item_position() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($stringVar: String)
        {
          complicatedArgs {
            stringListArgField(stringListArg: [$stringVar])
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn string_nonnull_to_string_list_in_item_position() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($stringVar: String!)
        {
          complicatedArgs {
            stringListArgField(stringListArg: [$stringVar])
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn complexinput_to_complexinput() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($complexVar: ComplexInput)
        {
          complicatedArgs {
            complexArgField(complexArg: $complexVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn complexinput_to_complexinput_in_field_position() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($boolVar: Boolean = false)
        {
          complicatedArgs {
            complexArgField(complexArg: { requiredArg: $boolVar })
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn boolean_nonnull_to_boolean_nonnull_in_directive() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($boolVar: Boolean!)
        {
          dog @include(if: $boolVar)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn int_to_int_nonnull() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($intArg: Int) {
          complicatedArgs {
            nonNullIntArgField(nonNullIntArg: $intArg)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Variable \"$intArg\" of type \"Int\" used in position expecting type \"Int!\"."]
    )
}

#[test]
fn int_to_int_nonnull_within_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "fragment nonNullIntArgFieldFrag on ComplicatedArgs {
          nonNullIntArgField(nonNullIntArg: $intArg)
        }
        query Query($intArg: Int) {
          complicatedArgs {
            ...nonNullIntArgFieldFrag
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Variable \"$intArg\" of type \"Int\" used in position expecting type \"Int!\"."]
    )
}

#[test]
fn int_to_int_nonnull_within_nested_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "fragment outerFrag on ComplicatedArgs {
          ...nonNullIntArgFieldFrag
        }
        fragment nonNullIntArgFieldFrag on ComplicatedArgs {
          nonNullIntArgField(nonNullIntArg: $intArg)
        }
        query Query($intArg: Int) {
          complicatedArgs {
            ...outerFrag
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Variable \"$intArg\" of type \"Int\" used in position expecting type \"Int!\"."]
    )
}

#[test]
fn string_over_boolean() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($stringVar: String) {
          complicatedArgs {
            booleanArgField(booleanArg: $stringVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
      "Variable \"$stringVar\" of type \"String\" used in position expecting type \"Boolean\"."
    ]
    )
}

#[test]
fn string_over_string_list() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($stringVar: String) {
          complicatedArgs {
            stringListArgField(stringListArg: $stringVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
      "Variable \"$stringVar\" of type \"String\" used in position expecting type \"[String]\"."
    ]
    )
}

#[test]
fn boolean_to_boolean_nonnull_in_directive() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($boolVar: Boolean) {
          dog @include(if: $boolVar)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
      "Variable \"$boolVar\" of type \"Boolean\" used in position expecting type \"Boolean!\"."
    ]
    )
}

#[test]
fn string_to_boolean_nonnull_in_directive() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($stringVar: String) {
          dog @include(if: $stringVar)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec![
      "Variable \"$stringVar\" of type \"String\" used in position expecting type \"Boolean!\"."
    ]
    )
}

#[test]
fn string_list_to_string_nonnull_list() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($stringListVar: [String])
        {
          complicatedArgs {
            stringListNonNullArgField(stringListNonNullArg: $stringListVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Variable \"$stringListVar\" of type \"[String]\" used in position expecting type \"[String!]\"."
    ])
}

#[test]
fn int_to_int_non_null_with_null_default_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($intVar: Int = null) {
          complicatedArgs {
            nonNullIntArgField(nonNullIntArg: $intVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages,
        vec!["Variable \"$intVar\" of type \"Int\" used in position expecting type \"Int!\"."]
    )
}

#[test]
fn int_to_int_non_null_with_default_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($intVar: Int = 1) {
          complicatedArgs {
            nonNullIntArgField(nonNullIntArg: $intVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn int_to_int_non_null_where_argument_with_default_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($intVar: Int) {
          complicatedArgs {
            nonNullFieldWithDefault(nonNullIntArg: $intVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn boolean_to_boolean_non_null_with_default_value() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($boolVar: Boolean = false) {
          dog @include(if: $boolVar)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}
