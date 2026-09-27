//! Input coercion of the built-in scalars (spec §3.5) and custom scalars, for JSON variables.

use super::harness::{
    assert_coerced, assert_forwarded, assert_invalid_json, assert_rejected,
    assert_rejected_by_validation,
};

/// `query ($v: <variable_type>) { <field>(input: $v) }`
fn operation(variable_type: &str, field: &str) -> String {
    format!("query ($v: {variable_type}) {{ {field}(input: $v) }}")
}

fn variables(value: &str) -> String {
    format!(r#"{{"v":{value}}}"#)
}

/// Asserts that `$v` given the JSON `value` is accepted and kept.
#[track_caller]
fn accepts(operation: &str, value: &str) {
    assert_coerced(operation, &variables(value), &variables(value));
}

/// Asserts that `$v` given the JSON `value` is rejected with `reason`.
#[track_caller]
fn rejects(operation: &str, value: &str, reason: &str) {
    assert_rejected(
        operation,
        &variables(value),
        &format!(r#"Variable "$v" has invalid value: {reason}"#),
    );
}

mod int {
    use super::*;

    fn op() -> String {
        operation("Int", "int")
    }

    /// graphql-js v17 scalars-test.ts:22-27, coerceInputValue-test.ts:126; v16 scalars-test.ts:21-23
    #[test]
    fn int_accepts_small_integers() {
        for value in ["1", "0", "-1"] {
            accepts(&op(), value);
        }
    }

    /// spec §3.5.1
    #[test]
    fn int_accepts_32_bit_bounds() {
        for value in ["2147483647", "-2147483648"] {
            accepts(&op(), value);
        }
    }

    /// graphql-js v17 scalars-test.ts:35 (BigInt, as a JSON integer); spec §3.5.1
    #[test]
    fn int_rejects_2_pow_31() {
        rejects(
            &op(),
            "2147483648",
            "Int cannot represent non 32-bit signed integer value: 2147483648",
        );
    }

    /// graphql-js v17 scalars-test.ts:38 (BigInt, as a JSON integer); spec §3.5.1
    #[test]
    fn int_rejects_below_minus_2_pow_31() {
        rejects(
            &op(),
            "-2147483649",
            "Int cannot represent non 32-bit signed integer value: -2147483649",
        );
    }

    /// graphql-js v17 scalars-test.ts:29; v16 scalars-test.ts:25
    #[test]
    fn int_rejects_9876504321() {
        rejects(
            &op(),
            "9876504321",
            "Int cannot represent non 32-bit signed integer value: 9876504321",
        );
    }

    /// graphql-js v17 scalars-test.ts:32; v16 scalars-test.ts:28
    #[test]
    fn int_rejects_minus_9876504321() {
        rejects(
            &op(),
            "-9876504321",
            "Int cannot represent non 32-bit signed integer value: -9876504321",
        );
    }

    /// spec §3.5.1: Int is 32-bit. graphql-js prints the JS double `9223372036854776000`.
    #[test]
    fn int_rejects_i64_max() {
        rejects(
            &op(),
            "9223372036854775807",
            "Int cannot represent non 32-bit signed integer value: 9223372036854775807",
        );
    }

    /// spec §3.5.1. graphql-js prints the JS double `9223372036854776000`.
    #[test]
    fn int_rejects_integer_above_i64() {
        rejects(
            &op(),
            "9223372036854775808",
            "Int cannot represent non 32-bit signed integer value: 9223372036854775808",
        );
    }

    /// spec §3.5.1. The JSON parser reads integers above u64 as floats, so this is rejected
    /// like any float.
    #[test]
    fn int_rejects_integer_above_u64() {
        rejects(
            &op(),
            "18446744073709551616",
            "Int cannot represent non-integer value: 18446744073709552000.0",
        );
    }

    #[test]
    fn int_rejects_json_floats() {
        for (value, printed) in [
            ("1.0", "1.0"),
            ("1e3", "1000.0"),
            ("1.5e1", "15.0"),
            ("-0", "0.0"),
            ("2147483648.0", "2147483648.0"),
            ("1.0000000000000001", "1.0"),
        ] {
            rejects(
                &op(),
                value,
                &format!("Int cannot represent non-integer value: {printed}"),
            );
        }
    }

    /// graphql-js v17 scalars-test.ts:41; v16 scalars-test.ts:31
    #[test]
    fn int_rejects_0_1() {
        rejects(&op(), "0.1", "Int cannot represent non-integer value: 0.1");
    }

    /// spec §3.5.1
    #[test]
    fn int_rejects_1_5() {
        rejects(&op(), "1.5", "Int cannot represent non-integer value: 1.5");
    }

    /// graphql-js v17 scalars-test.ts:60; v16 scalars-test.ts:50; spec §3.5.1
    #[test]
    fn int_rejects_numeric_string() {
        rejects(
            &op(),
            r#""123""#,
            r#"Int cannot represent non-integer value: "123""#,
        );
    }

    /// graphql-js v17 scalars-test.ts:57; v16 scalars-test.ts:47
    #[test]
    fn int_rejects_empty_string() {
        rejects(
            &op(),
            r#""""#,
            r#"Int cannot represent non-integer value: """#,
        );
    }

    /// graphql-js v17 scalars-test.ts:63; v16 scalars-test.ts:53
    #[test]
    fn int_rejects_false() {
        rejects(
            &op(),
            "false",
            "Int cannot represent non-integer value: false",
        );
    }

    /// graphql-js v17 scalars-test.ts:66; v16 scalars-test.ts:56
    #[test]
    fn int_rejects_true() {
        rejects(
            &op(),
            "true",
            "Int cannot represent non-integer value: true",
        );
    }

    /// graphql-js v17 scalars-test.ts:69; v16 scalars-test.ts:59
    #[test]
    fn int_rejects_list() {
        rejects(&op(), "[1]", "Int cannot represent non-integer value: [1]");
    }

    /// graphql-js v17 scalars-test.ts:72; v16 scalars-test.ts:62
    #[test]
    fn int_rejects_object() {
        rejects(
            &op(),
            r#"{"value":1}"#,
            "Int cannot represent non-integer value: { value: 1 }",
        );
    }

    /// graphql-js `inspect` prints nested objects to a depth of 2.
    #[test]
    fn int_error_truncates_deep_object() {
        rejects(
            &op(),
            r#"{"a":{"b":{"c":1}}}"#,
            "Int cannot represent non-integer value: { a: { b: [Object] } }",
        );
    }
}

mod float {
    use super::*;

    fn op() -> String {
        operation("Float", "float")
    }

    /// graphql-js v17 scalars-test.ts:213-220, coerceInputValue-test.ts:127;
    /// v16 scalars-test.ts:190-192; spec §3.5.2
    #[test]
    fn float_accepts_integers() {
        for value in ["1", "0", "-1"] {
            accepts(&op(), value);
        }
    }

    /// graphql-js v17 scalars-test.ts:216-217; v16 scalars-test.ts:193-194
    #[test]
    fn float_accepts_fractional_values() {
        for value in ["0.1", "3.141592653589793"] {
            assert_forwarded(&op(), &variables(value), &variables(value));
        }
    }

    /// graphql-js v17 scalars-test.ts:221 (BigInt, as a JSON integer)
    #[test]
    fn float_accepts_2_pow_53() {
        assert_forwarded(
            &op(),
            &variables("9007199254740992"),
            &variables("9007199254740992"),
        );
    }

    /// graphql-js v17 scalars-test.ts:229, coerceInputValue-test.ts:129 (BigInt, as a JSON
    /// integer); spec §3.5.2. Every finite value is accepted and forwarded with its digits.
    #[test]
    fn float_accepts_2_pow_53_plus_1() {
        assert_forwarded(
            &op(),
            &variables("9007199254740993"),
            &variables("9007199254740993"),
        );
    }

    /// spec §3.5.2
    #[test]
    fn float_accepts_integers_above_i64() {
        for value in ["9223372036854775808", "18446744073709551615"] {
            accepts(&op(), value);
        }
    }

    /// The JSON parser reads integers above u64 as floats.
    #[test]
    fn float_accepts_integer_above_u64() {
        accepts(&op(), "18446744073709551616");
    }

    /// spec §3.5.2
    #[test]
    fn float_accepts_extreme_finite_values() {
        for value in [
            "1e308",
            "1.7976931348623157e308",
            "-1.7976931348623157e308",
            "5e-324",
        ] {
            accepts(&op(), value);
        }
    }

    /// A value below the smallest f64 becomes 0, as with JavaScript's `JSON.parse`.
    #[test]
    fn float_accepts_underflow_to_zero() {
        assert_coerced(&op(), &variables("1e-400"), &variables("0.0"));
    }

    /// spec §3.5.2: not a finite value. Rejected by the JSON parser (HTTP 400).
    #[test]
    fn float_rejects_overflow_at_json_parse() {
        for value in ["1e309", "-1e309"] {
            assert_invalid_json(&variables(value));
        }
    }

    /// graphql-js v17 scalars-test.ts:232 (`2n ** 1024n`, as a JSON integer). Rejected by the
    /// JSON parser (HTTP 400).
    #[test]
    fn float_rejects_309_digit_integer_at_json_parse() {
        let two_pow_1024 = "179769313486231590772930519078902473361797697894230657273430081157732675805500963132708477322407536021120113879871393357658789768814416622492847430639474124377767893424865485276302219601246094119453082952085005768838150682342462881473913110540827237163350510684586298239947245938479716304835356329624224137216";
        assert_invalid_json(&variables(two_pow_1024));
    }

    /// `NaN` and `Infinity` are not JSON (some encoders, like Python's, emit them).
    #[test]
    fn float_rejects_nan_and_infinity_tokens() {
        for value in ["NaN", "Infinity", "-Infinity"] {
            assert_invalid_json(&variables(value));
        }
    }

    /// graphql-js v17 scalars-test.ts:242; v16 scalars-test.ts:209
    #[test]
    fn float_rejects_empty_string() {
        rejects(
            &op(),
            r#""""#,
            r#"Float cannot represent non numeric value: """#,
        );
    }

    /// graphql-js v17 scalars-test.ts:245; v16 scalars-test.ts:212
    #[test]
    fn float_rejects_numeric_string() {
        rejects(
            &op(),
            r#""123""#,
            r#"Float cannot represent non numeric value: "123""#,
        );
    }

    /// graphql-js v17 scalars-test.ts:248; v16 scalars-test.ts:215; spec §3.5.2
    #[test]
    fn float_rejects_decimal_string() {
        rejects(
            &op(),
            r#""123.5""#,
            r#"Float cannot represent non numeric value: "123.5""#,
        );
    }

    /// Strings are never parsed as numbers.
    #[test]
    fn float_rejects_nan_string() {
        rejects(
            &op(),
            r#""NaN""#,
            r#"Float cannot represent non numeric value: "NaN""#,
        );
    }

    /// graphql-js v17 scalars-test.ts:251; v16 scalars-test.ts:218
    #[test]
    fn float_rejects_false() {
        rejects(
            &op(),
            "false",
            "Float cannot represent non numeric value: false",
        );
    }

    /// graphql-js v17 scalars-test.ts:254; v16 scalars-test.ts:221
    #[test]
    fn float_rejects_true() {
        rejects(
            &op(),
            "true",
            "Float cannot represent non numeric value: true",
        );
    }

    /// graphql-js v17 scalars-test.ts:257; v16 scalars-test.ts:224
    #[test]
    fn float_rejects_list() {
        rejects(
            &op(),
            "[0.1]",
            "Float cannot represent non numeric value: [0.1]",
        );
    }

    /// graphql-js v17 scalars-test.ts:260; v16 scalars-test.ts:227
    #[test]
    fn float_rejects_object() {
        rejects(
            &op(),
            r#"{"value":0.1}"#,
            "Float cannot represent non numeric value: { value: 0.1 }",
        );
    }
}

mod string {
    use super::*;

    fn op() -> String {
        operation("String", "fieldWithNullableStringInput")
    }

    /// spec §3.5.3
    #[test]
    fn string_accepts_empty_string() {
        accepts(&op(), r#""""#);
    }

    /// spec §3.5.3
    #[test]
    fn string_accepts_unicode_and_escapes() {
        assert_coerced(
            &op(),
            r#"{"v":"héllo 世界 😀"}"#,
            r#"{"v":"héllo 世界 😀"}"#,
        );
        assert_coerced(&op(), r#"{"v":"é😀"}"#, r#"{"v":"é😀"}"#);
    }

    /// Quotes, backslashes and control characters stay escaped in the forwarded JSON.
    #[test]
    fn string_forwards_quotes_and_control_chars_escaped() {
        assert_forwarded(
            &op(),
            r#"{"v":"a\"b\\c\u0000\n"}"#,
            r#"{"v":"a\"b\\c\u0000\n"}"#,
        );
    }

    /// spec §3.5.3: only valid Unicode strings. Rejected by the JSON parser (HTTP 400).
    /// graphql-js accepts it, since JavaScript strings allow lone surrogates; the spec wins.
    #[test]
    fn string_rejects_lone_surrogate_at_json_parse() {
        assert_invalid_json(r#"{"v":"\uD800"}"#);
    }

    /// graphql-js v17 scalars-test.ts:369; v16 scalars-test.ts:328; spec §3.5.3
    #[test]
    fn string_rejects_integer() {
        rejects(&op(), "1", "String cannot represent a non string value: 1");
    }

    /// graphql-js v17 scalars-test.ts:375; v16 scalars-test.ts:334
    #[test]
    fn string_rejects_false() {
        rejects(
            &op(),
            "false",
            "String cannot represent a non string value: false",
        );
    }

    /// graphql-js v17 scalars-test.ts:378; v16 scalars-test.ts:337
    #[test]
    fn string_rejects_list() {
        rejects(
            &op(),
            r#"["foo"]"#,
            r#"String cannot represent a non string value: ["foo"]"#,
        );
    }

    /// graphql-js v17 scalars-test.ts:381; v16 scalars-test.ts:340
    #[test]
    fn string_rejects_object() {
        rejects(
            &op(),
            r#"{"value":"foo"}"#,
            r#"String cannot represent a non string value: { value: "foo" }"#,
        );
    }

    /// graphql-js v17 variables-test.ts:967; v16 variables-test.ts:754
    #[test]
    fn reports_error_for_array_passed_into_string_input() {
        assert_rejected(
            "query ($value: String!) { fieldWithNonNullableStringInput(input: $value) }",
            r#"{"value":[1,2,3]}"#,
            r#"Variable "$value" has invalid value: String cannot represent a non string value: [1, 2, 3]"#,
        );
    }

    /// graphql-js `inspect` prints at most 10 list items, so the error stays small.
    #[test]
    fn string_error_bounds_large_array() {
        let zeros = format!("[{}]", vec!["0"; 1000].join(","));
        rejects(
            &op(),
            &zeros,
            "String cannot represent a non string value: \
             [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, ... 990 more items]",
        );
    }
}

mod boolean {
    use super::*;

    fn op() -> String {
        operation("Boolean", "boolean")
    }

    /// graphql-js v17 scalars-test.ts:467-468; v16 scalars-test.ts:425-426
    #[test]
    fn boolean_accepts_true_and_false() {
        for value in ["true", "false"] {
            accepts(&op(), value);
        }
    }

    /// graphql-js v17 scalars-test.ts:476; v16 scalars-test.ts:434; spec §3.5.4
    #[test]
    fn boolean_rejects_0() {
        rejects(
            &op(),
            "0",
            "Boolean cannot represent a non boolean value: 0",
        );
    }

    /// graphql-js v17 scalars-test.ts:479; v16 scalars-test.ts:437; spec §3.5.4
    #[test]
    fn boolean_rejects_1() {
        rejects(
            &op(),
            "1",
            "Boolean cannot represent a non boolean value: 1",
        );
    }

    /// graphql-js v17 scalars-test.ts:485; v16 scalars-test.ts:443
    #[test]
    fn boolean_rejects_empty_string() {
        rejects(
            &op(),
            r#""""#,
            r#"Boolean cannot represent a non boolean value: """#,
        );
    }

    /// graphql-js v17 scalars-test.ts:488; v16 scalars-test.ts:446
    #[test]
    fn boolean_rejects_string_false() {
        rejects(
            &op(),
            r#""false""#,
            r#"Boolean cannot represent a non boolean value: "false""#,
        );
    }

    /// spec §3.5.4
    #[test]
    fn boolean_rejects_string_true() {
        rejects(
            &op(),
            r#""true""#,
            r#"Boolean cannot represent a non boolean value: "true""#,
        );
    }

    /// graphql-js v17 scalars-test.ts:491; v16 scalars-test.ts:449
    #[test]
    fn boolean_rejects_list() {
        rejects(
            &op(),
            "[false]",
            "Boolean cannot represent a non boolean value: [false]",
        );
    }

    /// graphql-js v17 scalars-test.ts:494; v16 scalars-test.ts:452
    #[test]
    fn boolean_rejects_object() {
        rejects(
            &op(),
            r#"{"value":false}"#,
            "Boolean cannot represent a non boolean value: { value: false }",
        );
    }
}

mod id {
    use super::*;

    fn op() -> String {
        operation("ID", "id")
    }

    /// graphql-js v17 scalars-test.ts:581-583; v16 scalars-test.ts:539-541; spec §3.5.5
    #[test]
    fn id_accepts_strings() {
        for value in [r#""""#, r#""1""#, r#""foo""#] {
            accepts(&op(), value);
        }
    }

    /// graphql-js v17 scalars-test.ts:584-589, coerceInputValue-test.ts:128;
    /// v16 scalars-test.ts:542-544; spec §3.5.5. Forwarded as a number, not as `"1"`.
    #[test]
    fn id_accepts_small_integers() {
        for value in ["1", "0", "-1"] {
            accepts(&op(), value);
        }
    }

    /// graphql-js v17 scalars-test.ts:592-593; v16 scalars-test.ts:547-548
    #[test]
    fn id_accepts_max_safe_integers() {
        for value in ["9007199254740991", "-9007199254740991"] {
            accepts(&op(), value);
        }
    }

    /// spec §3.5.5: IDs have no size limit, so the digits must not change.
    #[test]
    fn id_forwards_big_integers_with_exact_digits() {
        for value in ["90071992547409910", "12345678901234567890"] {
            assert_forwarded(&op(), &variables(value), &variables(value));
        }
    }

    /// spec §3.5.5
    #[test]
    fn id_rejects_json_floats() {
        for (value, printed) in [("4.0", "4.0"), ("1e3", "1000.0")] {
            rejects(
                &op(),
                value,
                &format!("ID cannot represent value: {printed}"),
            );
        }
    }

    /// spec §3.5.5
    #[test]
    fn id_rejects_4_5() {
        rejects(&op(), "4.5", "ID cannot represent value: 4.5");
    }

    /// graphql-js v17 scalars-test.ts:601; v16 scalars-test.ts:556
    #[test]
    fn id_rejects_0_1() {
        rejects(&op(), "0.1", "ID cannot represent value: 0.1");
    }

    /// graphql-js v17 scalars-test.ts:610; v16 scalars-test.ts:561; spec §3.5.5
    #[test]
    fn id_rejects_false() {
        rejects(&op(), "false", "ID cannot represent value: false");
    }

    /// graphql-js v17 scalars-test.ts:613; v16 scalars-test.ts:564
    #[test]
    fn id_rejects_list() {
        rejects(&op(), r#"["1"]"#, r#"ID cannot represent value: ["1"]"#);
    }

    /// graphql-js v17 scalars-test.ts:616; v16 scalars-test.ts:567
    #[test]
    fn id_rejects_object() {
        rejects(
            &op(),
            r#"{"value":"1"}"#,
            r#"ID cannot represent value: { value: "1" }"#,
        );
    }
}

/// Custom scalars are opaque to the router: any JSON value is accepted and forwarded unchanged.
mod custom_scalars {
    use super::*;

    fn op() -> String {
        operation("JSONScalar", "fieldWithJSONScalarInput")
    }

    /// graphql-js v17 variables-test.ts:1032
    #[test]
    fn allows_custom_scalars_with_non_embedded_variables() {
        assert_coerced(
            "query ($input: JSONScalar) { fieldWithJSONScalarInput(input: $input) }",
            r#"{"input":{"a":"foo","b":["bar"],"c":"baz"}}"#,
            r#"{"input":{"a":"foo","b":["bar"],"c":"baz"}}"#,
        );
    }

    /// graphql-js v17 validateInputValue-test.ts:87, coerceInputValue-test.ts:77
    #[test]
    fn custom_scalar_accepts_object() {
        accepts(&op(), r#"{"value":1}"#);
    }

    /// graphql-js v17 validateInputValue-test.ts:91, coerceInputValue-test.ts:81
    #[test]
    fn custom_scalar_accepts_object_with_null_member() {
        accepts(&op(), r#"{"value":null}"#);
    }

    /// spec §3.5: the router does not know a custom scalar's rules. A list is not wrapped
    /// or checked item by item.
    #[test]
    fn custom_scalar_accepts_any_json() {
        for value in ["1", "1.5", r#""x""#, "true", r#"[1,"a",null]"#, "[]", "{}"] {
            accepts(&op(), value);
        }
    }
}

/// spec §3.12 and §6.1.2. graphql-js's scalar-level `null`/`undefined` rows are ported against
/// `T!`, since a nullable variable accepts them.
mod null_and_omitted {
    use super::*;

    const NULLABLE_STRING: &str =
        "query ($value: String) { fieldWithNullableStringInput(input: $value) }";
    const NON_NULL_STRING: &str =
        "query ($value: String!) { fieldWithNonNullableStringInput(input: $value) }";

    /// graphql-js v17 variables-test.ts:781; v16 variables-test.ts:568
    #[test]
    fn allows_nullable_inputs_to_be_omitted_in_a_variable() {
        assert_coerced(NULLABLE_STRING, "{}", "{}");
    }

    /// graphql-js v17 variables-test.ts:809; v16 variables-test.ts:596
    #[test]
    fn allows_nullable_inputs_to_be_set_to_null_in_a_variable() {
        assert_coerced(NULLABLE_STRING, r#"{"value":null}"#, r#"{"value":null}"#);
    }

    /// graphql-js v17 variables-test.ts:824, scalars-test.ts:361; v16 variables-test.ts:611,
    /// scalars-test.ts:320
    #[test]
    fn allows_nullable_inputs_to_be_set_to_a_value_in_a_variable() {
        assert_coerced(NULLABLE_STRING, r#"{"value":"a"}"#, r#"{"value":"a"}"#);
    }

    /// graphql-js v17 variables-test.ts:920; v16 variables-test.ts:707
    #[test]
    fn allows_non_nullable_inputs_to_be_set_to_a_value_in_a_variable() {
        assert_coerced(NON_NULL_STRING, r#"{"value":"a"}"#, r#"{"value":"a"}"#);
    }

    /// graphql-js v17 variables-test.ts:883, scalars-test.ts:363; v16 variables-test.ts:670,
    /// scalars-test.ts:322
    #[test]
    fn does_not_allow_non_nullable_inputs_to_be_omitted_in_a_variable() {
        assert_rejected(
            NON_NULL_STRING,
            "{}",
            r#"Variable "$value" has invalid value: Expected a value of non-null type "String!" to be provided."#,
        );
    }

    /// graphql-js v17 variables-test.ts:901, scalars-test.ts:366; v16 variables-test.ts:688,
    /// scalars-test.ts:325
    #[test]
    fn does_not_allow_non_nullable_inputs_to_be_set_to_null_in_a_variable() {
        assert_rejected(
            NON_NULL_STRING,
            r#"{"value":null}"#,
            r#"Variable "$value" has invalid value: Expected value of non-null type "String!" not to be null."#,
        );
    }

    /// For each scalar: `T!` rejects `null` and an omitted value; `T` accepts both.
    #[track_caller]
    fn check_scalar(scalar: &str, field: &str) {
        let non_null = operation(&format!("{scalar}!"), field);
        rejects(
            &non_null,
            "null",
            &format!(r#"Expected value of non-null type "{scalar}!" not to be null."#),
        );
        assert_rejected(
            &non_null,
            "{}",
            &format!(
                r#"Variable "$v" has invalid value: Expected a value of non-null type "{scalar}!" to be provided."#
            ),
        );

        let nullable = operation(scalar, field);
        accepts(&nullable, "null");
        assert_coerced(&nullable, "{}", "{}");
    }

    /// graphql-js v17 scalars-test.ts:51, :54; v16 scalars-test.ts:41, :44
    #[test]
    fn int_null_and_omitted() {
        check_scalar("Int", "int");
    }

    /// graphql-js v17 scalars-test.ts:236, :239; v16 scalars-test.ts:203, :206
    #[test]
    fn float_null_and_omitted() {
        check_scalar("Float", "float");
    }

    /// graphql-js v17 scalars-test.ts:470, :473; v16 scalars-test.ts:428, :431
    #[test]
    fn boolean_null_and_omitted() {
        check_scalar("Boolean", "boolean");
    }

    /// graphql-js v17 scalars-test.ts:595, :598; v16 scalars-test.ts:550, :553
    #[test]
    fn id_null_and_omitted() {
        check_scalar("ID", "id");
    }

    /// spec §3.12 and §6.1.2
    #[test]
    fn custom_scalar_null_and_omitted() {
        check_scalar("JSONScalar", "fieldWithJSONScalarInput");
    }
}

/// spec §6.1.2: a default is coerced by the variable type when the variable is omitted.
mod defaults {
    use super::*;

    /// graphql-js v17 variables-test.ts:855; v16 variables-test.ts:642
    #[test]
    fn allows_non_nullable_variable_to_be_omitted_given_a_default() {
        assert_coerced(
            r#"query ($value: String! = "default") { fieldWithNullableStringInput(input: $value) }"#,
            "{}",
            r#"{"value":"default"}"#,
        );
    }

    /// graphql-js v17 variables-test.ts:869; v16 variables-test.ts:656
    #[test]
    fn allows_non_nullable_inputs_to_be_omitted_given_a_default() {
        assert_coerced(
            r#"query ($value: String = "default") { fieldWithNonNullableStringInput(input: $value) }"#,
            "{}",
            r#"{"value":"default"}"#,
        );
    }

    /// spec §3.5.5 and §6.1.2: an integer default is a valid ID.
    #[test]
    fn id_integer_default_used_when_omitted() {
        assert_coerced("query ($v: ID = 1) { id(input: $v) }", "{}", r#"{"v":1}"#);
    }

    /// spec §6.1.2 (the example in the algorithm's note)
    #[test]
    fn float_integer_default_used_when_omitted() {
        assert_coerced(
            "query ($v: Float = 1) { float(input: $v) }",
            "{}",
            r#"{"v":1}"#,
        );
    }

    /// spec §3.5.1 and §5.6.1
    #[test]
    fn int_out_of_range_default_is_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($v: Int = 2147483648) { int(input: $v) }",
            "ValuesOfCorrectType",
        );
    }
}
