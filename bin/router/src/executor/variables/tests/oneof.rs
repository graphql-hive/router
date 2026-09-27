//! Input coercion of OneOf input objects (spec §3.10.1) for JSON variables: exactly one field
//! must be given, and its value must not be `null`.

use super::harness::{assert_coerced, assert_rejected, assert_rejected_by_validation};

/// `OneOfTestInput @oneOf { a: String, b: Int }` (graphql-js `oneof-test.ts`, the spec's
/// `ExampleOneOfInputObject`)
const ONE_OF: &str = "query ($input: OneOfTestInput!) { test(input: $input) { a b } }";
/// `IntOneOf @oneOf { foo: Int, bar: Int }`
const INT_ONE_OF: &str = "query ($input: IntOneOf) { intOneOf(input: $input) }";
/// `[OneOfTestInput!]`
const ONE_OF_LIST: &str = "query ($input: [OneOfTestInput!]) { oneOfList(input: $input) }";
/// `OneOfHolder { one: OneOfTestInput, many: [OneOfTestInput!], name: String }`
const ONE_OF_HOLDER: &str = "query ($input: OneOfHolder) { oneOfHolder(input: $input) }";
/// `RecursiveOneOf @oneOf { leaf: Int, and: [RecursiveOneOf!], not: RecursiveOneOf }`
const RECURSIVE_ONE_OF: &str = "query ($input: RecursiveOneOf) { recursiveOneOf(input: $input) }";

/// The error for a OneOf value that doesn't have exactly one non-null field.
fn one_of_error(type_name: &str) -> String {
    format!(
        r#"Within OneOf Input Object type "{type_name}", exactly one field must be specified, and the value for that field must be non-null."#
    )
}

/// Asserts that `variables` is rejected with `Variable "$input" has invalid value<path>: <reason>`.
#[track_caller]
fn rejects(operation: &str, variables: &str, path: &str, reason: &str) {
    assert_rejected(
        operation,
        variables,
        &format!(r#"Variable "$input" has invalid value{path}: {reason}"#),
    );
}

/// Asserts that `variables` is accepted and forwarded unchanged.
#[track_caller]
fn accepts(operation: &str, variables: &str) {
    assert_coerced(operation, variables, variables);
}

mod valid_values {
    use super::*;

    /// graphql-js v17 oneof-test.ts:92, :113, validateInputValue-test.ts:391, :405,
    /// coerceInputValue-test.ts:203; v16 oneof-test.ts:93, :114, coerceInputValue-test.ts:319
    #[test]
    fn accepts_oneof_with_single_string_member() {
        accepts(ONE_OF, r#"{"input":{"a":"abc"}}"#);
    }

    /// spec §3.10.1
    #[test]
    fn accepts_oneof_with_single_int_member() {
        accepts(ONE_OF, r#"{"input":{"b":123}}"#);
    }

    /// `0`, `""` and `false` are not `null`.
    #[test]
    fn accepts_oneof_with_falsy_non_null_member() {
        accepts(ONE_OF, r#"{"input":{"b":0}}"#);
        accepts(ONE_OF, r#"{"input":{"a":""}}"#);
    }

    /// spec §6.1.2: an explicit `null` beats the default, and a `null` value has no fields to
    /// count.
    #[test]
    fn accepts_explicit_null_for_nullable_oneof_variable_with_default() {
        accepts(
            r#"query ($input: OneOfTestInput = {a: "abc"}) { test(input: $input) { a b } }"#,
            r#"{"input":null}"#,
        );
    }

    /// The rule applies to `@oneOf` types only.
    #[test]
    fn does_not_apply_oneof_rule_to_regular_input_object() {
        accepts(
            "query ($input: ExampleInputObject) { exampleInput(input: $input) }",
            r#"{"input":{"a":"abc","b":1}}"#,
        );
    }
}

/// spec §3.10.1: the value must have exactly one field, and it must not be `null`.
mod count_and_null {
    use super::*;

    /// spec §3.10.1
    #[test]
    fn rejects_oneof_with_no_fields() {
        rejects(
            ONE_OF,
            r#"{"input":{}}"#,
            "",
            &one_of_error("OneOfTestInput"),
        );
    }

    /// graphql-js v17 oneof-test.ts:158; v16 oneof-test.ts:137; spec §3.10.1
    #[test]
    fn rejects_oneof_with_two_non_null_fields() {
        rejects(
            ONE_OF,
            r#"{"input":{"a":"abc","b":123}}"#,
            "",
            &one_of_error("OneOfTestInput"),
        );
    }

    /// graphql-js v17 oneof-test.ts:182, validateInputValue-test.ts:395,
    /// coerceInputValue-test.ts:207; v16 oneof-test.ts:161, coerceInputValue-test.ts:324.
    /// A field given as `null` still counts.
    #[test]
    fn rejects_oneof_with_additional_null_field() {
        rejects(
            ONE_OF,
            r#"{"input":{"a":"abc","b":null}}"#,
            "",
            &one_of_error("OneOfTestInput"),
        );
    }

    /// spec §2.10.8: key order does not matter.
    #[test]
    fn rejects_oneof_with_null_field_listed_first() {
        rejects(
            ONE_OF,
            r#"{"input":{"b":null,"a":"abc"}}"#,
            "",
            &one_of_error("OneOfTestInput"),
        );
    }

    /// spec §3.10.1
    #[test]
    fn rejects_oneof_with_all_fields_null() {
        rejects(
            ONE_OF,
            r#"{"input":{"a":null,"b":null}}"#,
            "",
            &one_of_error("OneOfTestInput"),
        );
    }

    /// spec §3.10.1: the number of fields is checked before their values. graphql-js reports
    /// `… at .a: String cannot represent a non string value: 456` first.
    #[test]
    fn rejects_oneof_with_two_invalid_members() {
        rejects(
            ONE_OF,
            r#"{"input":{"a":456,"b":"xyz"}}"#,
            "",
            &one_of_error("OneOfTestInput"),
        );
    }

    /// graphql-js v17 oneof-test.ts:136, validateInputValue-test.ts:409,
    /// coerceInputValue-test.ts:211; v16 coerceInputValue-test.ts:336
    #[test]
    fn rejects_oneof_with_single_null_member() {
        rejects(
            ONE_OF,
            r#"{"input":{"a":null}}"#,
            " at .a",
            &one_of_error("OneOfTestInput"),
        );
    }
}

mod member_values_and_unknown_fields {
    use super::*;

    /// graphql-js v17 validateInputValue-test.ts:419, coerceInputValue-test.ts:227;
    /// v16 coerceInputValue-test.ts:347 (`NaN` there; JSON cannot carry it)
    #[test]
    fn rejects_oneof_member_of_wrong_type() {
        rejects(
            INT_ONE_OF,
            r#"{"input":{"foo":1.5}}"#,
            " at .foo",
            "Int cannot represent non-integer value: 1.5",
        );
    }

    /// spec §3.10.1
    #[test]
    fn rejects_non_object_for_oneof() {
        rejects(
            INT_ONE_OF,
            r#"{"input":123}"#,
            "",
            r#"Expected value of type "IntOneOf" to be an object, found: 123."#,
        );
    }

    /// spec §3.10.1
    #[test]
    fn rejects_array_for_non_list_oneof() {
        rejects(
            ONE_OF,
            r#"{"input":[{"a":"abc"}]}"#,
            "",
            r#"Expected value of type "OneOfTestInput" to be an object, found: [{ a: "abc" }]."#,
        );
    }

    /// graphql-js v17 validateInputValue-test.ts:446, coerceInputValue-test.ts:231;
    /// v16 coerceInputValue-test.ts:380. spec §3.10.1: every given field counts, so this is a
    /// OneOf error. graphql-js reports only the unknown field.
    #[test]
    fn rejects_unknown_field_next_to_valid_member() {
        rejects(
            INT_ONE_OF,
            r#"{"input":{"foo":123,"unknownField":123}}"#,
            "",
            &one_of_error("IntOneOf"),
        );
    }

    /// graphql-js v17 validateInputValue-test.ts:456; v16 coerceInputValue-test.ts:395
    /// (the variant without suggestions)
    #[test]
    fn rejects_oneof_with_only_unknown_field() {
        rejects(
            INT_ONE_OF,
            r#"{"input":{"bart":123}}"#,
            "",
            r#"Expected value of type "IntOneOf" not to include unknown field "bart", found: { bart: 123 }."#,
        );
    }
}

mod nested {
    use super::*;

    /// spec §3.10.1 and §3.11
    #[test]
    fn accepts_list_of_valid_oneofs() {
        accepts(ONE_OF_LIST, r#"{"input":[{"a":"abc"},{"b":1}]}"#);
    }

    #[test]
    fn rejects_list_item_with_two_oneof_fields() {
        rejects(
            ONE_OF_LIST,
            r#"{"input":[{"a":"x"},{"a":"x","b":1}]}"#,
            " at [1]",
            &one_of_error("OneOfTestInput"),
        );
    }

    #[test]
    fn rejects_list_item_with_null_oneof_member() {
        rejects(
            ONE_OF_LIST,
            r#"{"input":[{"a":null}]}"#,
            " at [0].a",
            &one_of_error("OneOfTestInput"),
        );
    }

    /// The router forwards the value as sent; the subgraph wraps it into a list.
    #[test]
    fn accepts_single_oneof_for_list_type_unwrapped() {
        accepts(ONE_OF_LIST, r#"{"input":{"a":"abc"}}"#);
    }

    #[test]
    fn rejects_single_empty_oneof_for_list_type() {
        rejects(
            ONE_OF_LIST,
            r#"{"input":{}}"#,
            "",
            &one_of_error("OneOfTestInput"),
        );
    }

    #[test]
    fn accepts_oneof_nested_in_input_object() {
        accepts(
            ONE_OF_HOLDER,
            r#"{"input":{"one":{"a":"abc"},"many":[{"b":1}]}}"#,
        );
    }

    #[test]
    fn accepts_null_for_nullable_nested_oneof_field() {
        accepts(ONE_OF_HOLDER, r#"{"input":{"one":null,"name":"x"}}"#);
    }

    #[test]
    fn rejects_nested_oneof_with_two_fields() {
        rejects(
            ONE_OF_HOLDER,
            r#"{"input":{"one":{"a":"abc","b":1}}}"#,
            " at .one",
            &one_of_error("OneOfTestInput"),
        );
    }

    #[test]
    fn rejects_nested_oneof_with_null_member() {
        rejects(
            ONE_OF_HOLDER,
            r#"{"input":{"one":{"a":null}}}"#,
            " at .one.a",
            &one_of_error("OneOfTestInput"),
        );
    }

    #[test]
    fn rejects_empty_oneof_in_nested_list() {
        rejects(
            ONE_OF_HOLDER,
            r#"{"input":{"many":[{"a":"x"},{}]}}"#,
            " at .many[1]",
            &one_of_error("OneOfTestInput"),
        );
    }

    /// An empty list is a non-null value.
    #[test]
    fn accepts_recursive_oneof_with_empty_list_member() {
        accepts(RECURSIVE_ONE_OF, r#"{"input":{"not":{"and":[]}}}"#);
    }

    #[test]
    fn rejects_invalid_oneof_deep_in_recursive_oneof() {
        rejects(
            RECURSIVE_ONE_OF,
            r#"{"input":{"and":[{"leaf":1},{"not":{}}]}}"#,
            " at .and[1].not",
            &one_of_error("RecursiveOneOf"),
        );
    }

    /// spec §6.1.2: coercion stops at the first error.
    #[test]
    fn many_invalid_oneof_items_yield_one_request_error() {
        let items = vec!["{}"; 1000].join(",");
        rejects(
            ONE_OF_LIST,
            &format!(r#"{{"input":[{items}]}}"#),
            " at [0]",
            &one_of_error("OneOfTestInput"),
        );
    }
}

/// spec §6.1.2: a default is coerced by the variable type when it is used. Invalid OneOf
/// defaults are rejected by validation, before coercion.
mod defaults {
    use super::*;

    /// `query ($input: OneOfTestInput! = <default>) { test(input: $input) { a b } }`
    fn one_of_with_default(default: &str) -> String {
        format!("query ($input: OneOfTestInput! = {default}) {{ test(input: $input) {{ a b }} }}")
    }

    /// `query ($input: TestOneOfInputObject = <default>) { fieldWithOneOfObjectInput(input: $input) }`
    fn test_one_of_with_default(default: &str) -> String {
        format!(
            "query ($input: TestOneOfInputObject = {default}) {{ fieldWithOneOfObjectInput(input: $input) }}"
        )
    }

    /// graphql-js v17 oneof-test.ts:49, valueFromAST-test.ts:239,
    /// coerceInputValue-test.ts:611; v16 oneof-test.ts:44
    #[test]
    fn accepts_good_oneof_default() {
        assert_coerced(
            &one_of_with_default(r#"{a: "abc"}"#),
            "{}",
            r#"{"input":{"a":"abc"}}"#,
        );
    }

    /// graphql-js v17 valueFromAST-test.ts:242, coerceInputValue-test.ts:614
    #[test]
    fn accepts_good_oneof_default_for_second_member() {
        assert_coerced(
            &test_one_of_with_default(r#"{b: "def"}"#),
            "{}",
            r#"{"input":{"b":"def"}}"#,
        );
    }

    /// graphql-js v17 oneof-test.ts:70, valueFromAST-test.ts:250,
    /// coerceInputValue-test.ts:620; v16 oneof-test.ts:65
    #[test]
    fn oneof_default_with_two_fields_is_rejected_by_validation() {
        assert_rejected_by_validation(
            &one_of_with_default(r#"{a: "abc", b: 123}"#),
            "ValuesOfCorrectType",
        );
    }

    /// graphql-js v17 valueFromAST-test.ts:245, coerceInputValue-test.ts:617
    #[test]
    fn oneof_default_with_additional_null_field_is_rejected_by_validation() {
        assert_rejected_by_validation(
            &test_one_of_with_default(r#"{a: "abc", b: null}"#),
            "ValuesOfCorrectType",
        );
    }

    /// graphql-js v17 valueFromAST-test.ts:248, coerceInputValue-test.ts:618
    #[test]
    fn oneof_default_with_single_null_member_is_rejected_by_validation() {
        assert_rejected_by_validation(
            &test_one_of_with_default("{a: null}"),
            "ValuesOfCorrectType",
        );
    }

    /// graphql-js v17 valueFromAST-test.ts:256, coerceInputValue-test.ts:622
    #[test]
    fn empty_oneof_default_is_rejected_by_validation() {
        assert_rejected_by_validation(&test_one_of_with_default("{}"), "ValuesOfCorrectType");
    }

    /// graphql-js v17 valueFromAST-test.ts:253, coerceInputValue-test.ts:621
    #[test]
    fn oneof_default_with_unknown_field_is_rejected_by_validation() {
        assert_rejected_by_validation(
            &test_one_of_with_default(r#"{a: "abc", c: "def"}"#),
            "ValuesOfCorrectType",
        );
    }
}

/// OneOf argument literals are checked by validation (spec §5.6.1 and §5.8.5).
mod literals {
    use super::*;

    /// graphql-js v17 validateInputValue-test.ts:1071; spec §3.10.1
    #[test]
    fn oneof_literal_with_two_fields_is_rejected_by_validation() {
        assert_rejected_by_validation(
            r#"{ test(input: {a: "abc", b: 123}) { a b } }"#,
            "ValuesOfCorrectType",
        );
    }

    /// spec §3.10.1
    #[test]
    fn oneof_literal_with_null_field_is_rejected_by_validation() {
        assert_rejected_by_validation("{ test(input: {a: null}) { a b } }", "ValuesOfCorrectType");
    }

    /// graphql-js v17 oneof-test.ts:206, validateInputValue-test.ts:1045. A nullable variable
    /// could make the field `null`.
    #[test]
    fn nullable_variable_in_oneof_literal_is_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($a: String) { test(input: {a: $a}) { a b } }",
            "VariablesInAllowedPosition",
        );
    }

    /// graphql-js v17 VariablesInAllowedPositionRule.ts:119: a default does not help, since the
    /// client can still send `null`.
    #[test]
    fn defaulted_nullable_variable_in_oneof_literal_is_rejected_by_validation() {
        assert_rejected_by_validation(
            r#"query ($a: String = "abc") { test(input: {a: $a}) { a b } }"#,
            "VariablesInAllowedPosition",
        );
    }
}

/// A non-null variable used as the field of a OneOf literal is coerced by its own type.
mod variables_in_literals {
    use super::*;

    const NON_NULL: &str = "query ($a: String!) { test(input: {a: $a}) { a b } }";

    /// graphql-js v17 validateInputValue-test.ts:1045
    #[test]
    fn forwards_non_null_variable_in_oneof_literal() {
        assert_coerced(NON_NULL, r#"{"a":"abc"}"#, r#"{"a":"abc"}"#);
    }

    /// graphql-js v17 validateInputValue-test.ts:1055
    #[test]
    fn rejects_null_for_non_null_variable_in_oneof_literal() {
        assert_rejected(
            NON_NULL,
            r#"{"a":null}"#,
            r#"Variable "$a" has invalid value: Expected value of non-null type "String!" not to be null."#,
        );
    }

    /// spec §3.10.1
    #[test]
    fn rejects_missing_non_null_variable_in_oneof_literal() {
        assert_rejected(
            NON_NULL,
            "{}",
            r#"Variable "$a" has invalid value: Expected a value of non-null type "String!" to be provided."#,
        );
    }
}
