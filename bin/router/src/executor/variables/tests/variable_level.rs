//! The `CoerceVariableValues` algorithm itself (spec §6.1.2), independent of the value types.

use super::harness::{assert_coerced, assert_rejected, assert_rejected_by_validation};

/// Every combination of {absent, `null`, value} × the kinds of variable definition.
mod decision_table {
    use super::*;

    const NULLABLE: &str = "query ($v: Int) { int(input: $v) }";
    const NULLABLE_WITH_DEFAULT: &str = "query ($v: Int = 5) { int(input: $v) }";
    const NULLABLE_WITH_NULL_DEFAULT: &str = "query ($v: Int = null) { int(input: $v) }";
    const NON_NULL: &str = "query ($v: Int!) { int(input: $v) }";
    const NON_NULL_WITH_DEFAULT: &str = "query ($v: Int! = 5) { int(input: $v) }";
    /// Invalid under validation (`ValuesOfCorrectType`); coercion must still hold up.
    const NON_NULL_WITH_NULL_DEFAULT: &str = "query ($v: Int! = null) { int(input: $v) }";
    const DEFAULTED_IN_NON_NULL_ARGUMENT: &str =
        r#"query ($v: String = "default") { fieldWithNonNullableStringInput(input: $v) }"#;

    /// spec §6.1.2
    #[test]
    fn nullable_absent_has_no_entry() {
        assert_coerced(NULLABLE, "{}", "{}");
    }

    /// spec §6.1.2
    #[test]
    fn nullable_null_is_null() {
        assert_coerced(NULLABLE, r#"{"v":null}"#, r#"{"v":null}"#);
    }

    /// spec §6.1.2
    #[test]
    fn nullable_value_is_kept() {
        assert_coerced(NULLABLE, r#"{"v":1}"#, r#"{"v":1}"#);
    }

    /// spec §6.1.2
    #[test]
    fn nullable_with_default_absent_uses_default() {
        assert_coerced(NULLABLE_WITH_DEFAULT, "{}", r#"{"v":5}"#);
    }

    /// spec §6.1.2: an explicit `null` is a provided value, so the default does not apply.
    #[test]
    fn nullable_with_default_null_is_null() {
        assert_coerced(NULLABLE_WITH_DEFAULT, r#"{"v":null}"#, r#"{"v":null}"#);
    }

    /// spec §6.1.2
    #[test]
    fn nullable_with_default_value_beats_default() {
        assert_coerced(NULLABLE_WITH_DEFAULT, r#"{"v":1}"#, r#"{"v":1}"#);
    }

    /// spec §6.1.2: a `null` default exists, so an absent variable becomes `null`.
    #[test]
    fn null_default_absent_is_null() {
        assert_coerced(NULLABLE_WITH_NULL_DEFAULT, "{}", r#"{"v":null}"#);
    }

    /// spec §6.1.2
    #[test]
    fn null_with_null_default_is_null() {
        assert_coerced(NULLABLE_WITH_NULL_DEFAULT, r#"{"v":null}"#, r#"{"v":null}"#);
    }

    /// spec §6.1.2
    #[test]
    fn value_with_null_default_is_kept() {
        assert_coerced(NULLABLE_WITH_NULL_DEFAULT, r#"{"v":1}"#, r#"{"v":1}"#);
    }

    /// spec §6.1.2
    #[test]
    fn non_null_absent_is_rejected() {
        assert_rejected(
            NON_NULL,
            "{}",
            r#"Variable "$v" has invalid value: Expected a value of non-null type "Int!" to be provided."#,
        );
    }

    /// spec §6.1.2
    #[test]
    fn non_null_null_is_rejected() {
        assert_rejected(
            NON_NULL,
            r#"{"v":null}"#,
            r#"Variable "$v" has invalid value: Expected value of non-null type "Int!" not to be null."#,
        );
    }

    /// spec §6.1.2
    #[test]
    fn non_null_value_is_kept() {
        assert_coerced(NON_NULL, r#"{"v":1}"#, r#"{"v":1}"#);
    }

    /// spec §6.1.2
    #[test]
    fn non_null_with_default_absent_uses_default() {
        assert_coerced(NON_NULL_WITH_DEFAULT, "{}", r#"{"v":5}"#);
    }

    /// spec §6.1.2: a default never replaces an explicit `null`.
    #[test]
    fn non_null_with_default_null_is_rejected() {
        assert_rejected(
            NON_NULL_WITH_DEFAULT,
            r#"{"v":null}"#,
            r#"Variable "$v" has invalid value: Expected value of non-null type "Int!" not to be null."#,
        );
    }

    /// spec §6.1.2
    #[test]
    fn value_beats_non_null_default() {
        assert_coerced(NON_NULL_WITH_DEFAULT, r#"{"v":7}"#, r#"{"v":7}"#);
    }

    /// spec §5.6.1 (Values of Correct Type): a `null` default for a non-null variable.
    #[test]
    fn non_null_variable_with_null_default_is_rejected_by_validation() {
        assert_rejected_by_validation(NON_NULL_WITH_NULL_DEFAULT, "ValuesOfCorrectType");
    }

    /// spec §6.1.2: the default is coerced by the variable type. graphql-js v17 values.ts:206-223.
    #[test]
    fn non_null_with_null_default_absent_is_rejected() {
        assert_rejected(
            NON_NULL_WITH_NULL_DEFAULT,
            "{}",
            r#"Variable "$v" has invalid default value: Expected value of non-null type "Int!" not to be null."#,
        );
    }

    /// spec §6.1.2
    #[test]
    fn non_null_with_null_default_null_is_rejected() {
        assert_rejected(
            NON_NULL_WITH_NULL_DEFAULT,
            r#"{"v":null}"#,
            r#"Variable "$v" has invalid value: Expected value of non-null type "Int!" not to be null."#,
        );
    }

    /// spec §6.1.2: the invalid default is never used when a value is provided.
    #[test]
    fn non_null_with_null_default_value_is_kept() {
        assert_coerced(NON_NULL_WITH_NULL_DEFAULT, r#"{"v":1}"#, r#"{"v":1}"#);
    }

    /// spec §5.8.5 and §6.4.1: allowed because the variable has a default.
    #[test]
    fn defaulted_variable_in_non_null_argument_absent_uses_default() {
        assert_coerced(DEFAULTED_IN_NON_NULL_ARGUMENT, "{}", r#"{"v":"default"}"#);
    }

    /// spec §5.8.5 and §6.4.1: the variable is nullable, so `null` passes coercion. The
    /// non-null argument then fails during execution in the subgraph, not in the router.
    #[test]
    fn defaulted_variable_in_non_null_argument_null_is_kept() {
        assert_coerced(
            DEFAULTED_IN_NON_NULL_ARGUMENT,
            r#"{"v":null}"#,
            r#"{"v":null}"#,
        );
    }

    /// spec §6.1.2
    #[test]
    fn defaulted_variable_in_non_null_argument_value_is_kept() {
        assert_coerced(
            DEFAULTED_IN_NON_NULL_ARGUMENT,
            r#"{"v":"a"}"#,
            r#"{"v":"a"}"#,
        );
    }
}

/// Default values are coerced by the variable type, and only when they are used.
mod defaults {
    use super::*;

    /// graphql-js v17 valueFromAST-test.ts:222, coerceInputValue-test.ts:547. graphql-js coerces
    /// the default to `{ foo: 7 }`; the router forwards it as written and the subgraph applies
    /// the input field default.
    #[test]
    fn input_object_default_is_kept_as_written() {
        assert_coerced(
            "query ($v: DefaultSevenInput = {}) { defaultSeven(input: $v) }",
            "{}",
            r#"{"v":{}}"#,
        );
    }

    /// graphql-js v17 values.ts:204-223
    #[test]
    fn invalid_default_is_ignored_when_a_value_is_provided() {
        assert_coerced(
            "query ($v: String = 123) { fieldWithNullableStringInput(input: $v) }",
            r#"{"v":"x"}"#,
            r#"{"v":"x"}"#,
        );
    }

    /// spec §6.1.2: a default that got past validation is still a request error when used.
    #[test]
    fn invalid_default_is_rejected_when_used() {
        assert_rejected(
            "query ($v: Int = [1]) { int(input: $v) }",
            "{}",
            r#"Variable "$v" has invalid default value: Int cannot represent non-integer value: [1]"#,
        );
    }
}

/// Coercion loops over the operation's variable definitions, not over the provided keys.
mod definitions {
    use super::*;

    /// spec §6.1.2: every definition is coerced, used or not.
    #[test]
    fn unused_variable_is_still_coerced() {
        assert_rejected(
            "query ($v: String, $unused: String!) { fieldWithNullableStringInput(input: $v) }",
            "{}",
            r#"Variable "$unused" has invalid value: Expected a value of non-null type "String!" to be provided."#,
        );
    }

    /// spec §2.1.8: names are case-sensitive, so `V` does not provide `$v`.
    #[test]
    fn variable_names_are_case_sensitive() {
        assert_rejected(
            "query ($v: String!) { fieldWithNonNullableStringInput(input: $v) }",
            r#"{"V":"a"}"#,
            r#"Variable "$v" has invalid value: Expected a value of non-null type "String!" to be provided."#,
        );
    }

    /// spec §5.8.1: rejected by validation, before coercion.
    #[test]
    fn duplicate_definitions_are_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($v: Int, $v: Int = 5) { int(input: $v) }",
            "UniqueVariableNames",
        );
    }

    /// graphql-js v17 variables-test.ts:1932
    #[test]
    fn does_not_expose_prototype_variable_names_when_omitted() {
        assert_coerced(
            "query ($toString: String) { fieldWithNullableStringInput(input: $toString) }",
            "{}",
            "{}",
        );
    }

    /// graphql-js v17 variables-test.ts:1938
    #[test]
    fn still_returns_provided_variables_with_colliding_names() {
        assert_coerced(
            "query ($toString: String) { fieldWithNullableStringInput(input: $toString) }",
            r#"{"toString":"value"}"#,
            r#"{"toString":"value"}"#,
        );
    }

    /// graphql-js v17 variables-test.ts:1932 (derived)
    #[test]
    fn prototype_named_required_variable_absent_is_rejected() {
        assert_rejected(
            "query ($constructor: String!) { fieldWithNonNullableStringInput(input: $constructor) }",
            "{}",
            r#"Variable "$constructor" has invalid value: Expected a value of non-null type "String!" to be provided."#,
        );
    }
}

/// Coercion stops at the first error: definition order first, then value order.
mod first_error {
    use super::*;

    const TWO_REQUIRED: &str = "query ($a: String!, $b: String!) { \
        a: fieldWithNonNullableStringInput(input: $a) \
        b: fieldWithNonNullableStringInput(input: $b) }";

    /// spec §6.1.2 (graphql-js reports both errors)
    #[test]
    fn first_error_in_definition_order() {
        assert_rejected(
            TWO_REQUIRED,
            r#"{"b":null}"#,
            r#"Variable "$a" has invalid value: Expected a value of non-null type "String!" to be provided."#,
        );
    }

    /// spec §6.1.2
    #[test]
    fn second_variable_invalid() {
        assert_rejected(
            TWO_REQUIRED,
            r#"{"a":"x","b":null}"#,
            r#"Variable "$b" has invalid value: Expected value of non-null type "String!" not to be null."#,
        );
    }

    /// spec §6.1.2: the order of the definitions decides, not the order of the JSON keys.
    #[test]
    fn definition_order_not_json_key_order() {
        assert_rejected(
            TWO_REQUIRED,
            r#"{"b":null,"a":null}"#,
            r#"Variable "$a" has invalid value: Expected value of non-null type "String!" not to be null."#,
        );
    }

    /// graphql-js v17 variables-test.ts:1458, v16 variables-test.ts:1090 (graphql-js reports
    /// all three errors)
    #[test]
    fn return_first_error_in_value_order() {
        assert_rejected(
            "query ($input: [String!]) { listNN(input: $input) }",
            r#"{"input":[0,1,2]}"#,
            r#"Variable "$input" has invalid value at [0]: String cannot represent a non string value: 0"#,
        );
    }
}

/// Variable definition syntax and types.
mod definition_syntax {
    use super::*;

    const DESCRIBED: &str =
        r#"query ("doc" $v: String!) { fieldWithNonNullableStringInput(input: $v) }"#;

    /// spec §2.11: variable definitions may have a description (new in September 2025).
    #[test]
    fn described_variable_absent_is_rejected() {
        assert_rejected(
            DESCRIBED,
            "{}",
            r#"Variable "$v" has invalid value: Expected a value of non-null type "String!" to be provided."#,
        );
    }

    /// spec §2.11
    #[test]
    fn described_variable_value_is_kept() {
        assert_coerced(DESCRIBED, r#"{"v":"a"}"#, r#"{"v":"a"}"#);
    }

    /// graphql-js v17 variables-test.ts:1269, v16 variables-test.ts:968. graphql-js catches
    /// it during execution; the router rejects it in validation, before coercion.
    #[test]
    fn does_not_allow_invalid_types_to_be_used_as_values() {
        assert_rejected_by_validation(
            "query ($input: Query!) { fieldWithObjectInput(input: $input) }",
            "VariablesAreInputTypes",
        );
    }

    /// graphql-js v17 variables-test.ts:1288, v16 variables-test.ts:987. graphql-js catches
    /// it during execution; the router rejects it in validation, before coercion.
    #[test]
    fn does_not_allow_unknown_types_to_be_used_as_values() {
        assert_rejected_by_validation(
            "query ($input: UnknownType!) { fieldWithObjectInput(input: $input) }",
            "KnownTypeNames",
        );
    }
}
