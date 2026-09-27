//! Input coercion of lists (spec §3.11) and non-null types (spec §3.12) for JSON variables.

use super::harness::{
    assert_coerced, assert_forwarded, assert_rejected, assert_rejected_by_validation,
};

/// `query ($value: <variable_type>) { fieldWithNullableStringInput }`: only the variable's
/// own type matters here.
fn operation(variable_type: &str) -> String {
    format!("query ($value: {variable_type}) {{ fieldWithNullableStringInput }}")
}

fn variables(value: &str) -> String {
    format!(r#"{{"value":{value}}}"#)
}

/// Asserts that `$value: <variable_type>` given the JSON `value` is accepted and kept.
#[track_caller]
fn accepts(variable_type: &str, value: &str) {
    assert_coerced(
        &operation(variable_type),
        &variables(value),
        &variables(value),
    );
}

/// Asserts that `$value: <variable_type>` given the JSON `value` is rejected with `reason` at
/// `path` (e.g. ` at [1]`, or empty for the value itself).
#[track_caller]
fn rejects(variable_type: &str, value: &str, path: &str, reason: &str) {
    assert_rejected(
        &operation(variable_type),
        &variables(value),
        &format!(r#"Variable "$value" has invalid value{path}: {reason}"#),
    );
}

/// spec §3.12. Absent and `null` values for `T!` are covered in `variable_level` and `scalars`.
mod non_null {
    use super::*;

    /// graphql-js v17 validateInputValue-test.ts:52, coerceInputValue-test.ts:52;
    /// v16 coerceInputValue-test.ts:56
    #[test]
    fn non_null_returns_no_error_for_non_null_value() {
        accepts("Int!", "1");
    }

    /// spec §3.12: the value is coerced by the wrapped type.
    #[test]
    fn non_null_value_is_coerced_by_the_wrapped_type() {
        rejects(
            "Int!",
            r#""1""#,
            "",
            r#"Int cannot represent non-integer value: "1""#,
        );
    }

    #[test]
    fn non_null_list_variable_missing() {
        assert_rejected(
            "query ($input: [String]!) { nnList(input: $input) }",
            "{}",
            r#"Variable "$input" has invalid value: Expected a value of non-null type "[String]!" to be provided."#,
        );
    }
}

/// The counter-examples of spec §3.12, which validation rejects before coercion.
mod spec_non_null_examples {
    use super::*;

    #[test]
    fn spec_non_null_argument_cannot_be_omitted() {
        assert_rejected_by_validation("{ withNonNullArg }", "ProvidedRequiredArguments");
    }

    #[test]
    fn spec_null_literal_for_non_null_argument() {
        assert_rejected_by_validation(
            "{ withNonNullArg(cannotBeNull: null) }",
            "ValuesOfCorrectType",
        );
    }

    #[test]
    fn spec_nullable_variable_for_non_null_argument() {
        assert_rejected_by_validation(
            "query withNullableVariable($var: String) { withNonNullArg(cannotBeNull: $var) }",
            "VariablesInAllowedPosition",
        );
    }
}

/// graphql-js `variables-test.ts` "Handles lists and nullability", and spec §3.12.1.
mod nullability_matrix {
    use super::*;

    const LIST: &str = "query ($input: [String]) { list(input: $input) }";
    const NN_LIST: &str = "query ($input: [String]!) { nnList(input: $input) }";
    const LIST_NN: &str = "query ($input: [String!]) { listNN(input: $input) }";
    const NN_LIST_NN: &str = "query ($input: [String!]!) { nnListNN(input: $input) }";

    fn input(value: &str) -> String {
        format!(r#"{{"input":{value}}}"#)
    }

    #[track_caller]
    fn accepts(operation: &str, value: &str) {
        assert_coerced(operation, &input(value), &input(value));
    }

    #[track_caller]
    fn rejects(operation: &str, value: &str, path: &str, reason: &str) {
        assert_rejected(
            operation,
            &input(value),
            &format!(r#"Variable "$input" has invalid value{path}: {reason}"#),
        );
    }

    /// graphql-js v17 variables-test.ts:1105; v16 variables-test.ts:804
    #[test]
    fn allows_lists_to_be_null() {
        accepts(LIST, "null");
    }

    /// graphql-js v17 variables-test.ts:1116; v16 variables-test.ts:815
    #[test]
    fn allows_lists_to_contain_values() {
        accepts(LIST, r#"["A"]"#);
    }

    /// graphql-js v17 variables-test.ts:1127; v16 variables-test.ts:826
    #[test]
    fn allows_lists_to_contain_null() {
        accepts(LIST, r#"["A",null,"B"]"#);
    }

    /// graphql-js v17 variables-test.ts:1138; v16 variables-test.ts:837
    #[test]
    fn does_not_allow_non_null_lists_to_be_null() {
        rejects(
            NN_LIST,
            "null",
            "",
            r#"Expected value of non-null type "[String]!" not to be null."#,
        );
    }

    /// graphql-js v17 variables-test.ts:1157; v16 variables-test.ts:856
    #[test]
    fn allows_non_null_lists_to_contain_values() {
        accepts(NN_LIST, r#"["A"]"#);
    }

    /// graphql-js v17 variables-test.ts:1168; v16 variables-test.ts:867
    #[test]
    fn allows_non_null_lists_to_contain_null() {
        accepts(NN_LIST, r#"["A",null,"B"]"#);
    }

    /// graphql-js v17 variables-test.ts:1179; v16 variables-test.ts:878
    #[test]
    fn allows_lists_of_non_nulls_to_be_null() {
        accepts(LIST_NN, "null");
    }

    /// graphql-js v17 variables-test.ts:1190; v16 variables-test.ts:889
    #[test]
    fn allows_lists_of_non_nulls_to_contain_values() {
        accepts(LIST_NN, r#"["A"]"#);
    }

    /// graphql-js v17 variables-test.ts:1201; v16 variables-test.ts:900
    #[test]
    fn does_not_allow_lists_of_non_nulls_to_contain_null() {
        rejects(
            LIST_NN,
            r#"["A",null,"B"]"#,
            " at [1]",
            r#"Expected value of non-null type "String!" not to be null."#,
        );
    }

    /// graphql-js v17 variables-test.ts:1220; v16 variables-test.ts:919
    #[test]
    fn does_not_allow_non_null_lists_of_non_nulls_to_be_null() {
        rejects(
            NN_LIST_NN,
            "null",
            "",
            r#"Expected value of non-null type "[String!]!" not to be null."#,
        );
    }

    /// graphql-js v17 variables-test.ts:1239; v16 variables-test.ts:938
    #[test]
    fn allows_non_null_lists_of_non_nulls_to_contain_values() {
        accepts(NN_LIST_NN, r#"["A"]"#);
    }

    /// graphql-js v17 variables-test.ts:1250; v16 variables-test.ts:949
    #[test]
    fn does_not_allow_non_null_lists_of_non_nulls_to_contain_null() {
        rejects(
            NN_LIST_NN,
            r#"["A",null,"B"]"#,
            " at [1]",
            r#"Expected value of non-null type "String!" not to be null."#,
        );
    }

    /// graphql-js v16 coerceInputValue-test.ts:556
    #[test]
    fn list_of_non_null_rejects_a_null_first_item() {
        super::rejects(
            "[Int!]",
            "[null]",
            " at [0]",
            r#"Expected value of non-null type "Int!" not to be null."#,
        );
    }

    /// spec §3.12.1 and §2.10.7
    #[test]
    fn non_null_list_accepts_an_empty_list() {
        accepts(NN_LIST, "[]");
    }

    /// spec §3.12.1 and §2.10.7
    #[test]
    fn non_null_list_of_non_null_accepts_an_empty_list() {
        accepts(NN_LIST_NN, "[]");
    }

    /// spec §3.11 and §3.12.1. The router forwards the value as sent; the subgraph wraps it
    /// into `["A"]`.
    #[test]
    fn non_null_list_of_non_null_accepts_a_single_value() {
        accepts(NN_LIST_NN, r#""A""#);
    }
}

/// spec §3.11: list input coercion.
mod list_coercion {
    use super::*;

    /// graphql-js v17 validateInputValue-test.ts:495, coerceInputValue-test.ts:286;
    /// v16 coerceInputValue-test.ts:450
    #[test]
    fn list_returns_no_error_for_a_valid_input() {
        accepts("[Int]", "[1,2,3]");
    }

    /// graphql-js v17 validateInputValue-test.ts:510, coerceInputValue-test.ts:300;
    /// v16 coerceInputValue-test.ts:466. Only the first error is reported (graphql-js also
    /// reports `true` at `[2]`).
    #[test]
    fn list_returns_an_error_for_an_invalid_input() {
        rejects(
            "[Int]",
            r#"[1,"b",true,4]"#,
            " at [1]",
            r#"Int cannot represent non-integer value: "b""#,
        );
    }

    /// graphql-js v17 validateInputValue-test.ts:523, coerceInputValue-test.ts:304;
    /// v16 coerceInputValue-test.ts:482. The router forwards `42`; the subgraph wraps it.
    #[test]
    fn list_accepts_a_non_list_value() {
        accepts("[Int]", "42");
    }

    /// graphql-js v17 validateInputValue-test.ts:527, coerceInputValue-test.ts:321;
    /// v16 coerceInputValue-test.ts:501. A single value has no index in the path.
    #[test]
    fn list_returns_an_error_for_a_non_list_invalid_value() {
        rejects(
            "[Int]",
            r#""INVALID""#,
            "",
            r#"Int cannot represent non-integer value: "INVALID""#,
        );
    }

    /// graphql-js v17 validateInputValue-test.ts:536, coerceInputValue-test.ts:325;
    /// v16 coerceInputValue-test.ts:512. `null` stays `null`, not `[null]`.
    #[test]
    fn list_returns_null_for_a_null_value() {
        accepts("[Int]", "null");
    }

    /// spec §2.10.7
    #[test]
    fn list_accepts_an_empty_list() {
        accepts("[Int]", "[]");
    }

    /// A string is a single item, not a list of characters.
    #[test]
    fn list_of_string_accepts_a_single_string() {
        accepts("[String]", r#""AB""#);
    }

    #[test]
    fn list_of_string_rejects_a_single_number() {
        rejects(
            "[String]",
            "123",
            "",
            "String cannot represent a non string value: 123",
        );
    }

    /// graphql-js v17 coerceInputValue-test.ts:308 (an array-like object, as JSON): an object is
    /// not a list.
    #[test]
    fn list_rejects_an_object_as_a_single_item() {
        rejects(
            "[Int]",
            r#"{"length":1}"#,
            "",
            "Int cannot represent non-integer value: { length: 1 }",
        );
    }

    /// spec §3.11
    #[test]
    fn list_rejects_a_list_as_an_item() {
        rejects(
            "[Int]",
            "[[1]]",
            " at [0]",
            "Int cannot represent non-integer value: [1]",
        );
    }

    /// spec §3.5.1: a value is wrapped into a list, never unwrapped from one.
    #[test]
    fn non_list_type_rejects_a_list() {
        rejects(
            "Int",
            "[1]",
            "",
            "Int cannot represent non-integer value: [1]",
        );
    }
}

/// spec §3.11: nested lists. The router forwards values as sent; the subgraph does the
/// wrapping shown in the spec's table.
mod nested_lists {
    use super::*;

    /// graphql-js v17 validateInputValue-test.ts:544, coerceInputValue-test.ts:333;
    /// v16 coerceInputValue-test.ts:521
    #[test]
    fn nested_list_returns_no_error_for_a_valid_input() {
        accepts("[[Int]]", "[[1],[2,3]]");
    }

    /// graphql-js v17 validateInputValue-test.ts:548, coerceInputValue-test.ts:337;
    /// v16 coerceInputValue-test.ts:526
    #[test]
    fn nested_list_accepts_a_non_list_value() {
        accepts("[[Int]]", "42");
    }

    /// graphql-js v17 validateInputValue-test.ts:552, coerceInputValue-test.ts:341;
    /// v16 coerceInputValue-test.ts:531
    #[test]
    fn nested_list_returns_null_for_a_null_value() {
        accepts("[[Int]]", "null");
    }

    /// graphql-js v17 validateInputValue-test.ts:556, coerceInputValue-test.ts:345;
    /// v16 coerceInputValue-test.ts:536
    #[test]
    fn nested_list_accepts_nested_non_list_values() {
        accepts("[[Int]]", "[1,2,3]");
    }

    /// graphql-js v17 validateInputValue-test.ts:560, coerceInputValue-test.ts:349;
    /// v16 coerceInputValue-test.ts:541
    #[test]
    fn nested_list_accepts_nested_null_values() {
        accepts("[[Int]]", "[42,[null],null]");
    }

    /// spec §3.11 (new in September 2025)
    #[test]
    fn nested_list_keeps_a_null_item_null() {
        accepts("[[Int]]", "[1,null,3]");
    }

    /// spec §3.11 (new in September 2025)
    #[test]
    fn nested_list_rejects_an_incorrect_item_value() {
        rejects(
            "[[Int]]",
            r#"[[1],["b"]]"#,
            " at [1][0]",
            r#"Int cannot represent non-integer value: "b""#,
        );
    }

    /// Wrapping a single item adds no index to the path.
    #[test]
    fn nested_list_wrapped_item_error_has_no_inner_index() {
        rejects(
            "[[Int]]",
            r#"[1,"b"]"#,
            " at [1]",
            r#"Int cannot represent non-integer value: "b""#,
        );
    }

    #[test]
    fn nested_list_rejects_a_non_list_invalid_value() {
        rejects(
            "[[Int]]",
            r#""b""#,
            "",
            r#"Int cannot represent non-integer value: "b""#,
        );
    }

    /// spec §2.10.7
    #[test]
    fn nested_list_accepts_an_empty_inner_list() {
        accepts("[[Int]]", "[[]]");
    }
}

/// spec §3.12.1: the non-null rules apply at every level of a nested list.
mod nested_lists_with_non_null {
    use super::*;

    #[test]
    fn nested_list_of_non_null_rejects_a_null_item() {
        rejects(
            "[[Int!]]",
            "[[1],[null]]",
            " at [1][0]",
            r#"Expected value of non-null type "Int!" not to be null."#,
        );
    }

    #[test]
    fn nested_list_of_non_null_allows_a_null_inner_list() {
        accepts("[[Int!]]", "[[1],null]");
    }

    #[test]
    fn nested_list_of_non_null_wraps_items_and_allows_null() {
        accepts("[[Int!]]", "[1,null]");
    }

    #[test]
    fn list_of_non_null_lists_rejects_a_null_inner_list() {
        rejects(
            "[[Int]!]",
            "[[1],null]",
            " at [1]",
            r#"Expected value of non-null type "[Int]!" not to be null."#,
        );
    }

    #[test]
    fn list_of_non_null_lists_allows_null_items() {
        accepts("[[Int]!]", "[[null]]");
    }

    #[test]
    fn fully_non_null_nested_list_accepts_an_empty_inner_list() {
        accepts("[[Int!]!]!", "[[]]");
    }

    #[test]
    fn fully_non_null_nested_list_accepts_a_scalar() {
        accepts("[[Int!]!]!", "1");
    }

    #[test]
    fn fully_non_null_nested_list_rejects_a_null_inner_list() {
        rejects(
            "[[Int!]!]!",
            "[1,[2],null]",
            " at [2]",
            r#"Expected value of non-null type "[Int!]!" not to be null."#,
        );
    }

    #[test]
    fn fully_non_null_nested_list_rejects_null() {
        rejects(
            "[[Int!]!]!",
            "null",
            "",
            r#"Expected value of non-null type "[[Int!]!]!" not to be null."#,
        );
    }
}

/// spec §6.1.2: a list-typed default is coerced by the variable type when it is used.
mod defaults {
    use super::*;

    /// The router inserts the default as written; the subgraph wraps it into `[1]`.
    #[test]
    fn list_default_single_value_is_inserted_as_is() {
        assert_coerced(&operation("[Int] = 1"), "{}", r#"{"value":1}"#);
    }

    #[test]
    fn non_null_list_default_with_null_item_is_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($value: [Int!]! = [1, null]) { fieldWithNullableStringInput }",
            "ValuesOfCorrectType",
        );
    }
}

/// Large lists: linear work, one error, and a bounded error message.
mod large_inputs {
    use super::*;

    fn ints(count: usize) -> Vec<String> {
        (0..count).map(|n| n.to_string()).collect()
    }

    #[test]
    fn large_list_of_valid_items_is_forwarded_unchanged() {
        let value = format!("[{}]", ints(100_000).join(","));
        assert_forwarded(&operation("[Int]"), &variables(&value), &variables(&value));
    }

    /// spec §6.1.2: coercion stops at the first error.
    #[test]
    fn large_list_of_nulls_yields_one_error() {
        let value = format!("[{}]", vec!["null"; 100_000].join(","));
        rejects(
            "[Int!]",
            &value,
            " at [0]",
            r#"Expected value of non-null type "Int!" not to be null."#,
        );
    }

    #[test]
    fn large_list_reports_the_last_index() {
        let mut items = ints(99_999);
        items.push(r#""x""#.to_string());
        rejects(
            "[Int]",
            &format!("[{}]", items.join(",")),
            " at [99999]",
            r#"Int cannot represent non-integer value: "x""#,
        );
    }

    /// graphql-js `inspect` prints at most 10 list items, so the error stays small.
    #[test]
    fn invalid_item_message_is_bounded() {
        let inner: Vec<String> = (1..=100_000).map(|n| n.to_string()).collect();
        rejects(
            "[Int]",
            &format!("[[{}]]", inner.join(",")),
            " at [0]",
            "Int cannot represent non-integer value: \
             [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, ... 99990 more items]",
        );
    }
}
