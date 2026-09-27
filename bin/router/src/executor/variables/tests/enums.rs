//! Input coercion of enums (spec §3.9) for JSON variables. An enum value is sent as a JSON
//! string holding the value's name.

use super::harness::{assert_coerced, assert_rejected, assert_rejected_by_validation};

/// `query ($value: <variable_type>) { <field>(input: $value) }`
fn operation(variable_type: &str, field: &str) -> String {
    format!("query ($value: {variable_type}) {{ {field}(input: $value) }}")
}

fn variables(value: &str) -> String {
    format!(r#"{{"value":{value}}}"#)
}

/// Asserts that `$value` given the JSON `value` is accepted and kept.
#[track_caller]
fn accepts(operation: &str, value: &str) {
    assert_coerced(operation, &variables(value), &variables(value));
}

/// Asserts that `$value` given the JSON `value` is rejected with `reason` at `path`
/// (e.g. ` at [1]`, or empty for the value itself).
#[track_caller]
fn rejects_at(operation: &str, value: &str, path: &str, reason: &str) {
    assert_rejected(
        operation,
        &variables(value),
        &format!(r#"Variable "$value" has invalid value{path}: {reason}"#),
    );
}

#[track_caller]
fn rejects(operation: &str, value: &str, reason: &str) {
    rejects_at(operation, value, "", reason);
}

mod valid_names {
    use super::*;

    /// graphql-js v17 validateInputValue-test.ts:131, coerceInputValue-test.ts:110;
    /// v16 coerceInputValue-test.ts:144
    #[test]
    fn returns_no_error_for_a_known_enum_name_foo() {
        accepts(&operation("FooBarEnum", "fooBar"), r#""FOO""#);
    }

    /// graphql-js v17 validateInputValue-test.ts:133, coerceInputValue-test.ts:112;
    /// v16 coerceInputValue-test.ts:147
    #[test]
    fn returns_no_error_for_a_known_enum_name_bar() {
        accepts(&operation("FooBarEnum", "fooBar"), r#""BAR""#);
    }

    /// graphql-js v17 enumType-test.ts:284; v16 enumType-test.ts:250
    #[test]
    fn accepts_json_string_as_enum_variable() {
        assert_coerced(
            "query ($color: Color!) { colorEnum(fromEnum: $color) }",
            r#"{"color":"BLUE"}"#,
            r#"{"color":"BLUE"}"#,
        );
    }

    /// graphql-js v17 enumType-test.ts:363; v16 enumType-test.ts:329. The router forwards the
    /// name; internal values only exist in the subgraph.
    #[test]
    fn enum_value_may_have_an_internal_value_of_0() {
        accepts(&operation("Color", "color"), r#""RED""#);
    }

    /// graphql-js v17 variables-test.ts:729; v16 variables-test.ts:516 (the literals as
    /// variables, forwarded by name)
    #[test]
    fn allows_custom_enum_values_as_inputs() {
        for name in [
            "NULL",
            "UNDEFINED",
            "NAN",
            "FALSE",
            "CUSTOM",
            "DEFAULT_VALUE",
        ] {
            accepts(
                &operation("TestEnum", "fieldWithEnumInput"),
                &format!(r#""{name}""#),
            );
        }
    }

    /// graphql-js v17 variables-test.ts:751; v16 variables-test.ts:538. The name `"NULL"` is
    /// not a JSON `null`.
    #[test]
    fn allows_non_nullable_inputs_to_have_null_as_enum_custom_value() {
        accepts(
            &operation("TestEnum!", "fieldWithNonNullableEnumInput"),
            r#""NULL""#,
        );
    }

    /// spec §3.9: deprecated values are still valid input.
    #[test]
    fn deprecated_enum_value_is_accepted() {
        accepts(&operation("Color", "color"), r#""OLD_BLUE""#);
    }

    /// graphql-js v17 enumType-test.ts:379; v16 enumType-test.ts:345
    #[test]
    fn enum_inputs_may_be_nullable() {
        assert_coerced(&operation("Color", "color"), "{}", "{}");
    }

    /// spec §3.9 and §6.1.2
    #[test]
    fn nullable_enum_explicit_null_is_forwarded() {
        accepts(&operation("Color", "color"), "null");
    }
}

mod unknown_names {
    use super::*;

    fn color() -> String {
        operation("Color", "color")
    }

    /// graphql-js v17 validateInputValue-test.ts:136
    #[test]
    fn returns_an_error_for_unknown_enum_value() {
        rejects(
            &operation("FooBarEnum", "fooBar"),
            r#""UNKNOWN""#,
            r#"Value "UNKNOWN" does not exist in "FooBarEnum" enum."#,
        );
    }

    /// graphql-js v17 validateInputValue-test.ts:145, coerceInputValue-test.ts:116;
    /// v16 coerceInputValue-test.ts:151 (the variant without suggestions)
    #[test]
    fn returns_an_error_for_misspelled_enum_value() {
        rejects(
            &operation("FooBarEnum", "fooBar"),
            r#""foo""#,
            r#"Value "foo" does not exist in "FooBarEnum" enum."#,
        );
    }

    /// graphql-js v17 enumType-test.ts:229; v16 enumType-test.ts:195 (literal as a variable)
    #[test]
    fn does_not_accept_values_with_incorrect_casing() {
        rejects(
            &color(),
            r#""green""#,
            r#"Value "green" does not exist in "Color" enum."#,
        );
    }

    /// graphql-js v17 enumType-test.ts:198; v16 enumType-test.ts:181 (literal as a variable)
    #[test]
    fn does_not_accept_values_not_in_the_enum() {
        rejects(
            &color(),
            r#""GREENISH""#,
            r#"Value "GREENISH" does not exist in "Color" enum."#,
        );
    }

    /// A value of another enum is not a value of this one.
    #[test]
    fn value_of_another_enum_is_rejected() {
        rejects(
            &color(),
            r#""FOO""#,
            r#"Value "FOO" does not exist in "Color" enum."#,
        );
    }

    /// spec §2.10.6: the string `"null"` is not the enum value `NULL`.
    #[test]
    fn string_null_is_not_enum_value_null() {
        rejects(
            &operation("TestEnum", "fieldWithEnumInput"),
            r#""null""#,
            r#"Value "null" does not exist in "TestEnum" enum."#,
        );
    }

    /// spec §2.10.6: the string `"false"` is not the enum value `FALSE`.
    #[test]
    fn string_false_is_not_enum_value_false() {
        rejects(
            &operation("TestEnum", "fieldWithEnumInput"),
            r#""false""#,
            r#"Value "false" does not exist in "TestEnum" enum."#,
        );
    }

    /// spec §2.1.8: names match exactly.
    #[test]
    fn enum_name_whitespace_is_not_trimmed() {
        rejects(
            &color(),
            r#""RED ""#,
            r#"Value "RED " does not exist in "Color" enum."#,
        );
    }

    /// spec §2.1.8: `RЕD` with a Cyrillic `Е` is a different name.
    #[test]
    fn enum_name_homoglyph_is_rejected() {
        rejects(
            &color(),
            r#""RЕD""#,
            "Value \"R\u{0415}D\" does not exist in \"Color\" enum.",
        );
    }

    /// spec §2.1.8
    #[test]
    fn empty_string_is_rejected() {
        rejects(
            &color(),
            r#""""#,
            r#"Value "" does not exist in "Color" enum."#,
        );
    }

    /// graphql-js prints the unknown name as it is, without escaping it.
    #[test]
    fn unknown_name_is_echoed_raw() {
        rejects(
            &color(),
            r#""RED\"""#,
            r#"Value "RED"" does not exist in "Color" enum."#,
        );
    }

    /// Strings are never read as a value's position.
    #[test]
    fn numeric_string_is_rejected() {
        rejects(
            &color(),
            r#""2""#,
            r#"Value "2" does not exist in "Color" enum."#,
        );
    }
}

mod non_string_values {
    use super::*;

    /// graphql-js v17 validateInputValue-test.ts:196, coerceInputValue-test.ts:120;
    /// v16 coerceInputValue-test.ts:164
    #[test]
    fn returns_an_error_for_incorrect_value_type_number() {
        rejects(
            &operation("FooBarEnum", "fooBar"),
            "123",
            r#"Enum "FooBarEnum" cannot represent non-string value: 123."#,
        );
    }

    /// graphql-js v17 validateInputValue-test.ts:203, coerceInputValue-test.ts:121;
    /// v16 coerceInputValue-test.ts:173
    #[test]
    fn returns_an_error_for_incorrect_value_type_object() {
        rejects(
            &operation("FooBarEnum", "fooBar"),
            r#"{"field":"value"}"#,
            r#"Enum "FooBarEnum" cannot represent non-string value: { field: "value" }."#,
        );
    }

    /// graphql-js v17 enumType-test.ts:312; v16 enumType-test.ts:278
    #[test]
    fn does_not_accept_internal_value_as_enum_variable() {
        assert_rejected(
            "query ($color: Color!) { colorEnum(fromEnum: $color) }",
            r#"{"color":2}"#,
            r#"Variable "$color" has invalid value: Enum "Color" cannot represent non-string value: 2."#,
        );
    }

    /// spec §3.9
    #[test]
    fn boolean_is_rejected() {
        rejects(
            &operation("Color", "color"),
            "true",
            r#"Enum "Color" cannot represent non-string value: true."#,
        );
    }

    /// spec §2.10.6 and §3.9: JSON `false` is not the enum value `FALSE`.
    #[test]
    fn boolean_false_is_not_enum_value_false() {
        rejects(
            &operation("TestEnum", "fieldWithEnumInput"),
            "false",
            r#"Enum "TestEnum" cannot represent non-string value: false."#,
        );
    }

    /// spec §3.12: JSON `null` is not the enum value `NULL`.
    #[test]
    fn json_null_for_non_null_enum() {
        rejects(
            &operation("TestEnum!", "fieldWithNonNullableEnumInput"),
            "null",
            r#"Expected value of non-null type "TestEnum!" not to be null."#,
        );
    }

    /// spec §3.9
    #[test]
    fn array_for_non_list_enum() {
        rejects(
            &operation("Color", "color"),
            r#"["RED"]"#,
            r#"Enum "Color" cannot represent non-string value: ["RED"]."#,
        );
    }
}

/// spec §3.11
mod lists {
    use super::*;

    fn colors() -> String {
        operation("[Color]", "colors")
    }

    #[test]
    fn list_of_enum_names() {
        accepts(&colors(), r#"["RED","BLUE"]"#);
    }

    /// The router forwards the value as sent; the subgraph wraps it into `["RED"]`.
    #[test]
    fn single_enum_name_for_list() {
        accepts(&colors(), r#""RED""#);
    }

    #[test]
    fn list_item_unknown_name() {
        rejects_at(
            &colors(),
            r#"["RED","PURPLE"]"#,
            " at [1]",
            r#"Value "PURPLE" does not exist in "Color" enum."#,
        );
    }

    #[test]
    fn single_unknown_name_for_list_has_no_index() {
        rejects(
            &colors(),
            r#""PURPLE""#,
            r#"Value "PURPLE" does not exist in "Color" enum."#,
        );
    }

    #[test]
    fn list_item_non_string() {
        rejects_at(
            &colors(),
            r#"["RED",1]"#,
            " at [1]",
            r#"Enum "Color" cannot represent non-string value: 1."#,
        );
    }

    #[test]
    fn nested_array_item_for_list_of_enum() {
        rejects_at(
            &colors(),
            r#"[["RED"]]"#,
            " at [0]",
            r#"Enum "Color" cannot represent non-string value: ["RED"]."#,
        );
    }

    #[test]
    fn null_item_in_nullable_enum_list() {
        accepts(&colors(), r#"["RED",null]"#);
    }

    /// spec §3.11 and §3.12
    #[test]
    fn null_item_in_non_null_enum_list() {
        rejects_at(
            &operation("[Color!]", "colorsNN"),
            r#"["RED",null]"#,
            " at [1]",
            r#"Expected value of non-null type "Color!" not to be null."#,
        );
    }

    /// The router forwards the value as sent; the subgraph wraps it into `[["RED"],["BLUE"]]`.
    #[test]
    fn nested_enum_list_wraps_items() {
        accepts(&operation("[[Color]]", "nestedColors"), r#"["RED","BLUE"]"#);
    }

    #[test]
    fn nested_enum_list_inner_wrong_case() {
        rejects_at(
            &operation("[[Color]]", "nestedColors"),
            r#"[["RED"],["red"]]"#,
            " at [1][0]",
            r#"Value "red" does not exist in "Color" enum."#,
        );
    }

    /// spec §6.1.2: only the first error is reported (graphql-js also reports `[1]`).
    #[test]
    fn enum_list_many_bad_items_reports_first() {
        rejects_at(
            &colors(),
            r#"["PURPLE","ORANGE"]"#,
            " at [0]",
            r#"Value "PURPLE" does not exist in "Color" enum."#,
        );
    }
}

/// spec §6.1.2: a default is coerced by the variable type when the variable is omitted.
mod defaults {
    use super::*;

    /// graphql-js v17 coerceInputValue-test.ts:471, valueFromAST-test.ts:114
    #[test]
    fn enum_literal_default_is_inserted() {
        assert_coerced(
            "query ($value: Color = RED) { color(input: $value) }",
            "{}",
            r#"{"value":"RED"}"#,
        );
    }

    /// graphql-js v17 coerceInputValue-test.ts:476-477, valueFromAST-test.ts:119-120
    #[test]
    fn enum_default_named_null_is_inserted_as_string() {
        assert_coerced(
            "query ($value: TestEnum! = NULL) { fieldWithNonNullableEnumInput(input: $value) }",
            "{}",
            r#"{"value":"NULL"}"#,
        );
    }

    /// graphql-js v17 coerceInputValue-test.ts:475, valueFromAST-test.ts:118; spec §2.10.6
    #[test]
    fn null_keyword_default_is_not_enum_null() {
        assert_coerced(
            "query ($value: TestEnum = null) { fieldWithEnumInput(input: $value) }",
            "{}",
            r#"{"value":null}"#,
        );
    }

    /// The router inserts the default as written; the subgraph wraps it into `["RED"]`.
    #[test]
    fn single_enum_default_for_list() {
        assert_coerced(
            "query ($value: [Color] = RED) { colors(input: $value) }",
            "{}",
            r#"{"value":"RED"}"#,
        );
    }

    /// graphql-js v17 enumType-test.ts:184, coerceInputValue-test.ts:474,
    /// valueFromAST-test.ts:117; v16 enumType-test.ts:167. spec §3.9: a string literal is
    /// not an enum value.
    #[test]
    fn string_literal_default_is_rejected_by_validation() {
        assert_rejected_by_validation(
            r#"query ($value: Color = "RED") { color(input: $value) }"#,
            "ValuesOfCorrectType",
        );
    }

    /// spec §2.10.6
    #[test]
    fn boolean_literal_default_is_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($value: Color = true) { color(input: $value) }",
            "ValuesOfCorrectType",
        );
    }

    /// graphql-js v17 enumType-test.ts:258, coerceInputValue-test.ts:473,
    /// valueFromAST-test.ts:116; v16 enumType-test.ts:224
    #[test]
    fn int_literal_default_is_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($value: Color = 3) { color(input: $value) }",
            "ValuesOfCorrectType",
        );
    }

    /// graphql-js v17 validateInputValue-test.ts:717 (literal)
    #[test]
    fn unknown_enum_literal_default_is_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($value: Color = PURPLE) { color(input: $value) }",
            "ValuesOfCorrectType",
        );
    }

    /// spec §5.6.1
    #[test]
    fn unknown_enum_item_in_non_null_list_default_is_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($value: [Color]! = [PURPLE]) { colors(input: $value) }",
            "ValuesOfCorrectType",
        );
    }

    /// spec §5.6.1
    #[test]
    fn list_literal_default_for_enum_is_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($value: Color = [RED]) { color(input: $value) }",
            "ValuesOfCorrectType",
        );
    }
}

/// spec §6.1: variables are coerced the same way for every operation type.
mod operation_types {
    use super::*;

    const MUTATION: &str = "mutation ($color: Color!) { favoriteEnum(color: $color) }";
    const SUBSCRIPTION: &str = "subscription ($color: Color!) { subscribeToEnum(color: $color) }";

    /// graphql-js v17 enumType-test.ts:293; v16 enumType-test.ts:259
    #[test]
    fn accepts_enum_literals_as_input_arguments_to_mutations() {
        assert_coerced(MUTATION, r#"{"color":"GREEN"}"#, r#"{"color":"GREEN"}"#);
    }

    /// graphql-js v17 enumType-test.ts:302; v16 enumType-test.ts:268
    #[test]
    fn accepts_enum_literals_as_input_arguments_to_subscriptions() {
        assert_coerced(SUBSCRIPTION, r#"{"color":"GREEN"}"#, r#"{"color":"GREEN"}"#);
    }

    #[test]
    fn rejects_invalid_enum_variable_on_mutation() {
        assert_rejected(
            MUTATION,
            r#"{"color":"PURPLE"}"#,
            r#"Variable "$color" has invalid value: Value "PURPLE" does not exist in "Color" enum."#,
        );
    }

    #[test]
    fn rejects_invalid_enum_variable_on_subscription() {
        assert_rejected(
            SUBSCRIPTION,
            r#"{"color":"PURPLE"}"#,
            r#"Variable "$color" has invalid value: Value "PURPLE" does not exist in "Color" enum."#,
        );
    }
}

/// A variable's type must fit the enum position; validation checks this before coercion.
mod variable_position {
    use super::*;

    /// graphql-js v17 enumType-test.ts:327; v16 enumType-test.ts:293
    #[test]
    fn does_not_accept_string_variables_as_enum_input() {
        assert_rejected_by_validation(
            "query ($color: String!) { colorEnum(fromEnum: $color) }",
            "VariablesInAllowedPosition",
        );
    }

    /// graphql-js v17 enumType-test.ts:345; v16 enumType-test.ts:311
    #[test]
    fn does_not_accept_internal_value_variable_as_enum_input() {
        assert_rejected_by_validation(
            "query ($color: Int!) { colorEnum(fromEnum: $color) }",
            "VariablesInAllowedPosition",
        );
    }
}
