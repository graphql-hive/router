use std::collections::{HashMap, HashSet};

use crate::{
    ast::{AstNodeWithName, OperationVisitor, OperationVisitorContext},
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
pub struct VariablesInAllowedPosition<'doc> {
    spreads: HashMap<Scope<'doc>, HashSet<&'doc str>>,
    variable_usages: HashMap<Scope<'doc>, Vec<(&'doc str, &'doc Type, bool)>>,
    variable_defs: HashMap<Scope<'doc>, Vec<&'doc VariableDefinition>>,
    current_scope: Option<Scope<'doc>>,
}

impl<'doc> VariablesInAllowedPosition<'doc> {
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
        from: &Scope<'doc>,
        var_defs: &[&VariableDefinition],
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        visited: &mut HashSet<Scope<'doc>>,
    ) {
        if visited.contains(from) {
            return;
        }

        visited.insert(from.clone());

        let usages = match self.variable_usages.get(from) {
            Some(usages) => usages.as_slice(),
            None => &[],
        };
        for (var_name, location_type, has_default) in usages {
            let Some(var_def) = var_defs.iter().find(|var_def| var_def.name == *var_name) else {
                continue;
            };

            let has_non_null_default = var_def
                .default_value
                .as_ref()
                .is_some_and(|v| !matches!(v, Value::Null));

            // https://spec.graphql.org/draft/#sec-All-Variable-Usages-are-Allowed
            // If a variable definition has a non-null default value (e.g., $foo: String = "hi"),
            // the variable can be used where a non-null type is expected,
            // because when the variable is omitted, the default kicks in.
            // An explicit null default ($foo: String = null) doesn't get this treatment,
            // since null doesn't satisfy a non-null position.
            let variable_type = match &var_def.var_type {
                Type::NonNullType(_) => var_def.var_type.clone(),
                t if has_non_null_default => Type::NonNullType(Box::new(t.clone())),
                t => t.clone(),
            };

            // A default at the usage location permits a nullable variable in a
            // non-null position, so ignore the location's outer nullability.
            let effective_location_type = match (has_default, location_type) {
                (true, Type::NonNullType(inner)) => inner.as_ref(),
                _ => location_type,
            };

            if !visitor_context
                .schema
                .is_subtype(&variable_type, effective_location_type)
            {
                user_context.report_error(ValidationError {
                    error_code: self.error_code(),
                    message: format!(
                        "Variable \"${}\" of type \"{}\" used in position expecting type \"{}\".",
                        var_name, variable_type, location_type,
                    ),
                    locations: vec![var_def.position],
                });
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
pub enum Scope<'doc> {
    Operation(Option<&'doc str>),
    Fragment(&'doc str),
}

impl<'doc> OperationVisitor<'doc, ValidationErrorContext> for VariablesInAllowedPosition<'doc> {
    fn leave_document(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'doc>,
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
        _: &mut OperationVisitorContext<'doc>,
        _: &mut ValidationErrorContext,
        fragment_definition: &'doc crate::static_graphql::query::FragmentDefinition,
    ) {
        self.current_scope = Some(Scope::Fragment(&fragment_definition.name));
    }

    fn enter_operation_definition(
        &mut self,
        _: &mut OperationVisitorContext<'doc>,
        _: &mut ValidationErrorContext,
        operation_definition: &'doc crate::static_graphql::query::OperationDefinition,
    ) {
        self.current_scope = Some(Scope::Operation(operation_definition.node_name()));
    }

    fn enter_fragment_spread(
        &mut self,
        _: &mut OperationVisitorContext<'doc>,
        _: &mut ValidationErrorContext,
        fragment_spread: &'doc crate::static_graphql::query::FragmentSpread,
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
        _: &mut OperationVisitorContext<'doc>,
        _: &mut ValidationErrorContext,
        variable_definition: &'doc VariableDefinition,
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
        visitor_context: &mut OperationVisitorContext<'doc>,
        _: &mut ValidationErrorContext,
        variable_name: &'doc str,
    ) {
        if let (Some(scope), Some(input_type)) = (
            &self.current_scope,
            visitor_context.current_input_type_literal(),
        ) {
            let has_default = visitor_context.current_input_type_has_default();
            self.variable_usages
                .entry(scope.clone())
                .or_default()
                .push((variable_name, input_type, has_default));
        }
    }
}

impl ValidationRule for VariablesInAllowedPosition<'_> {
    fn error_code(&self) -> &'static str {
        "VariablesInAllowedPosition"
    }

    fn visitor<'doc>(&self) -> super::ValidationVisitor<'doc> {
        Box::new(VariablesInAllowedPosition::new())
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
            nonNullFieldWithDefault(arg: $intVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn list_of_non_null_enum_with_single_enum_default_value() {
    use crate::validation::test_utils::*;

    // A single-value default on a list-typed variable is valid per
    // GraphQL spec (single values coerce to a one-item list). The router used to
    // reject this with the malformed type "T!!" because the expected-type
    // computation dropped the list wrapper and double-wrapped NonNull.
    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($enumListVar: [FurColor!] = BROWN) {
          complicatedArgs {
            enumListArgField(enumListArg: $enumListVar)
          }
        }",
        // Augment the schema with a list-of-non-null-enum argument.
        &(TEST_SCHEMA.replace(
            "enumArgField(enumArg: FurColor): String",
            "enumArgField(enumArg: FurColor): String\n  enumListArgField(enumListArg: [FurColor!]): String",
        )),
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages, Vec::<&String>::new());
}

#[test]
fn list_of_non_null_with_default_value_used_in_non_null_list_position() {
    use crate::validation::test_utils::*;

    // A list variable with a non-null default value should be usable in a
    // non-null list position. Before the fix, the expected type for the
    // subtype check was "T!!" (invalid) instead of "[T!]!".
    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($enumListVar: [FurColor!] = BROWN) {
          complicatedArgs {
            requiredEnumListArgField(enumListArg: $enumListVar)
          }
        }",
        &(TEST_SCHEMA.replace(
            "enumArgField(enumArg: FurColor): String",
            "enumArgField(enumArg: FurColor): String\n  requiredEnumListArgField(enumListArg: [FurColor!]!): String",
        )),
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages, Vec::<&String>::new());
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

#[test]
fn nullable_enum_to_non_null_enum_with_default_on_argument() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($currency: FurColor) {
          complicatedArgs {
            enumArgFieldWithDefault(enumArg: $currency)
          }
        }",
        &(TEST_SCHEMA.replace(
            "enumArgField(enumArg: FurColor): String",
            "enumArgField(enumArg: FurColor): String\n  enumArgFieldWithDefault(enumArg: FurColor! = BROWN): String",
        )),
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn nullable_int_to_non_null_int_with_default_on_argument() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($intVar: Int) {
          complicatedArgs {
            nonNullFieldWithDefault(arg: $intVar)
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn nullable_int_to_non_null_input_field_with_default() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($intVar: Int) {
          set(input: { value: $intVar })
        }",
        "input Input {
          value: Int! = 1
        }
        type Query {
          set(input: Input): String
        }",
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn string_to_non_null_int_with_default_on_argument() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(VariablesInAllowedPosition::new()));
    let errors = test_operation_with_schema(
        "query Query($stringVar: String) {
          complicatedArgs {
            nonNullFieldWithDefault(arg: $stringVar)
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
            "Variable \"$stringVar\" of type \"String\" used in position expecting type \"Int!\"."
        ]
    )
}
