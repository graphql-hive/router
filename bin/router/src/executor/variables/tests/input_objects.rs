//! Input coercion of input objects (spec §3.10) for JSON variables.

use super::harness::{assert_coerced, assert_rejected, assert_rejected_by_validation};

/// graphql-js `variables-test.ts`: `TestInputObject { a: String, b: [String], c: String!,
/// d: ComplexScalar, e: FaultyScalar }`
const TEST_INPUT: &str = "query ($input: TestInputObject) { fieldWithObjectInput(input: $input) }";
/// `TestNestedInputObject { na: TestInputObject!, nb: String! }`
const NESTED_INPUT: &str =
    "query ($input: TestNestedInputObject) { fieldWithNestedInputObject(input: $input) }";
/// graphql-js type-level tests: `IntInputObject { foo: Int!, bar: Int, nested: IntInputObject,
/// deepObject: DeepObject }`
const INT_INPUT: &str = "query ($v: IntInputObject) { intInput(input: $v) }";
/// spec §3.10: `ExampleInputObject { a: String, b: Int! }`
const EXAMPLE_INPUT: &str = "query ($var: ExampleInputObject) { exampleInput(input: $var) }";
/// `DefaultedRequired { x: Int! = 5, y: String }`
const DEFAULTED_REQUIRED: &str = "query ($v: DefaultedRequired) { defaultedRequired(input: $v) }";
/// `Wrappers { list: [Int!], nested: [ExampleInputObject!], inner: ExampleInputObject }`
const WRAPPERS: &str = "query ($v: Wrappers) { wrappers(input: $v) }";
/// `NNListField { items: [String]! }`
const NN_LIST_FIELD: &str = "query ($v: NNListField) { nnListField(input: $v) }";
/// `[LengthObject]`, with `LengthObject { length: Int }`
const LENGTH_OBJECTS: &str = "query ($v: [LengthObject]) { lengthObjects(input: $v) }";
/// `Filter { and: [Filter!], name: String }`
const FILTER: &str = "query ($f: Filter) { filter(input: $f) }";

/// Asserts that `variables` is rejected with `Variable "$<name>" has invalid value<path>: <reason>`.
#[track_caller]
fn rejects(operation: &str, variables: &str, name: &str, path: &str, reason: &str) {
    assert_rejected(
        operation,
        variables,
        &format!(r#"Variable "${name}" has invalid value{path}: {reason}"#),
    );
}

/// Asserts that `variables` is accepted and forwarded unchanged.
#[track_caller]
fn accepts(operation: &str, variables: &str) {
    assert_coerced(operation, variables, variables);
}

/// spec §3.10: only a JSON object is an input object value.
mod non_object_values {
    use super::*;

    /// graphql-js `variables-test.ts` "errors on incorrect type"
    #[test]
    fn errors_on_incorrect_type() {
        rejects(
            TEST_INPUT,
            r#"{"input":"foo bar"}"#,
            "input",
            "",
            r#"Expected value of type "TestInputObject" to be an object, found: "foo bar"."#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "for GraphQLInputObject > returns an error for a
    /// non-object type"; `coerceInputValue-test.ts` "invalid for a non-object type", "returns an
    /// error for a non-object type"
    #[test]
    fn input_object_rejects_number() {
        rejects(
            INT_INPUT,
            r#"{"v":123}"#,
            "v",
            "",
            r#"Expected value of type "IntInputObject" to be an object, found: 123."#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "returns error when supplied with an array";
    /// `coerceInputValue-test.ts` "invalid when supplied with an array", "returns an error for an
    /// array type"
    #[test]
    fn input_object_rejects_array() {
        rejects(
            INT_INPUT,
            r#"{"v":[{"foo":123},{"bar":456}]}"#,
            "v",
            "",
            r#"Expected value of type "IntInputObject" to be an object, found: [{ foo: 123 }, { bar: 456 }]."#,
        );
    }

    /// spec §3.10
    #[test]
    fn input_object_rejects_boolean() {
        rejects(
            INT_INPUT,
            r#"{"v":true}"#,
            "v",
            "",
            r#"Expected value of type "IntInputObject" to be an object, found: true."#,
        );
    }

    /// spec §3.10: a list is never wrapped for a type that isn't a list.
    #[test]
    fn input_object_rejects_empty_list() {
        rejects(
            INT_INPUT,
            r#"{"v":[]}"#,
            "v",
            "",
            r#"Expected value of type "IntInputObject" to be an object, found: []."#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "returns error when a nested input object is
    /// supplied with an array"; `coerceInputValue-test.ts` "invalid when a nested input object is
    /// supplied with an array"
    #[test]
    fn nested_input_object_rejects_array() {
        rejects(
            INT_INPUT,
            r#"{"v":{"foo":123,"nested":[{"foo":123},{"bar":456}]}}"#,
            "v",
            " at .nested",
            r#"Expected value of type "IntInputObject" to be an object, found: [{ foo: 123 }, { bar: 456 }]."#,
        );
    }

    /// graphql-js `coerceInputValue-test.ts` "returns an error for an array type on a nested field"
    #[test]
    fn nested_deep_object_rejects_list() {
        rejects(
            INT_INPUT,
            r#"{"v":{"foo":1,"deepObject":[1,2,3]}}"#,
            "v",
            " at .deepObject",
            r#"Expected value of type "DeepObject" to be an object, found: [1, 2, 3]."#,
        );
    }
}

/// spec §3.10: a field is required only if it is non-null and has no default. Field defaults
/// are applied by the subgraph, so the router forwards the value without them.
mod required_fields_and_defaults {
    use super::*;

    /// graphql-js `variables-test.ts` "errors on omission of nested non-null"
    #[test]
    fn errors_on_omission_of_nested_non_null() {
        rejects(
            TEST_INPUT,
            r#"{"input":{"a":"foo","b":"bar"}}"#,
            "input",
            "",
            r#"Expected value of type "TestInputObject" to include required field "c", found: { a: "foo", b: "bar" }."#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "for GraphQLInputObject > returns error for a
    /// missing required field"; `coerceInputValue-test.ts` "invalid for a missing required field",
    /// "returns error for a missing required field"
    #[test]
    fn returns_error_for_a_missing_required_field() {
        rejects(
            INT_INPUT,
            r#"{"v":{"bar":123}}"#,
            "v",
            "",
            r#"Expected value of type "IntInputObject" to include required field "foo", found: { bar: 123 }."#,
        );
    }

    /// spec §3.10
    #[test]
    fn empty_object_missing_required_field() {
        rejects(
            TEST_INPUT,
            r#"{"input":{}}"#,
            "input",
            "",
            r#"Expected value of type "TestInputObject" to include required field "c", found: {}."#,
        );
    }

    /// spec §3.10
    #[test]
    fn missing_non_null_nested_object_field() {
        rejects(
            NESTED_INPUT,
            r#"{"input":{"nb":"x"}}"#,
            "input",
            "",
            r#"Expected value of type "TestNestedInputObject" to include required field "na", found: { nb: "x" }."#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "for GraphQLInputObject > returns no error for a
    /// valid input"; `coerceInputValue-test.ts` "for GraphQLInputObject > returns no error for a
    /// valid input"
    #[test]
    fn returns_no_error_for_a_valid_input() {
        accepts(INT_INPUT, r#"{"v":{"foo":123}}"#);
    }

    /// graphql-js `validateInputValue-test.ts` "for GraphQLInputObject with default value > no
    /// error for no errors for valid input value"; `coerceInputValue-test.ts` "returns no errors
    /// for valid input value"
    #[test]
    fn field_default_valid_input_value() {
        accepts(
            "query ($v: DefaultSevenInput) { defaultSeven(input: $v) }",
            r#"{"v":{"foo":5}}"#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "for GraphQLInputObject with default value > no
    /// error for object with default value"; `coerceInputValue-test.ts` "returns object with
    /// default value". graphql-js coerces to `{ foo: 7 }`.
    #[test]
    fn field_default_omitted_not_injected() {
        accepts(
            "query ($v: DefaultSevenInput) { defaultSeven(input: $v) }",
            r#"{"v":{}}"#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "for GraphQLInputObject with default value > no
    /// error for null as value"; `coerceInputValue-test.ts` "returns null as value"
    #[test]
    fn null_field_default_omitted_not_injected() {
        accepts(
            "query ($v: DefaultNullInput) { defaultNull(input: $v) }",
            r#"{"v":{}}"#,
        );
    }

    /// spec §3.10: an explicit `null` is kept; the default only replaces an absent field.
    #[test]
    fn explicit_null_beats_field_default() {
        accepts(
            "query ($v: DefaultSevenInput) { defaultSeven(input: $v) }",
            r#"{"v":{"foo":null}}"#,
        );
    }

    /// spec §3.10: `x: Int! = 5` has a default, so it is not required.
    #[test]
    fn non_null_field_with_default_may_be_omitted() {
        accepts(DEFAULTED_REQUIRED, r#"{"v":{}}"#);
        accepts(DEFAULTED_REQUIRED, r#"{"v":{"y":"s"}}"#);
    }

    /// spec §3.10: a default does not make `null` valid for a non-null field.
    #[test]
    fn non_null_field_with_default_rejects_null() {
        rejects(
            DEFAULTED_REQUIRED,
            r#"{"v":{"x":null}}"#,
            "v",
            " at .x",
            r#"Expected value of non-null type "Int!" not to be null."#,
        );
    }

    /// spec §3.10
    #[test]
    fn field_with_default_type_checked_when_present() {
        rejects(
            DEFAULTED_REQUIRED,
            r#"{"v":{"x":"5"}}"#,
            "v",
            " at .x",
            r#"Int cannot represent non-integer value: "5""#,
        );
    }
}

/// spec §3.10
mod explicit_null_fields {
    use super::*;

    /// graphql-js `variables-test.ts` "errors on null for nested non-null"
    #[test]
    fn errors_on_null_for_nested_non_null() {
        rejects(
            TEST_INPUT,
            r#"{"input":{"a":"foo","b":"bar","c":null}}"#,
            "input",
            " at .c",
            r#"Expected value of non-null type "String!" not to be null."#,
        );
    }

    #[test]
    fn explicit_null_for_nullable_fields_kept() {
        accepts(
            TEST_INPUT,
            r#"{"input":{"a":null,"b":null,"c":"baz","d":null}}"#,
        );
    }

    #[test]
    fn null_for_nullable_nested_object() {
        accepts(INT_INPUT, r#"{"v":{"foo":1,"nested":null}}"#);
    }

    /// spec §3.10 and §3.12
    #[test]
    fn null_for_non_null_nested_object() {
        rejects(
            NESTED_INPUT,
            r#"{"input":{"na":null,"nb":"x"}}"#,
            "input",
            " at .na",
            r#"Expected value of non-null type "TestInputObject!" not to be null."#,
        );
    }
}

/// spec §3.10: a field the type does not define is an error.
mod unknown_fields {
    use super::*;

    /// graphql-js `variables-test.ts` "errors on addition of unknown input field"
    #[test]
    fn errors_on_addition_of_unknown_input_field() {
        rejects(
            TEST_INPUT,
            r#"{"input":{"a":"foo","b":"bar","c":"baz","extra":"dog"}}"#,
            "input",
            "",
            r#"Expected value of type "TestInputObject" not to include unknown field "extra", found: { a: "foo", b: "bar", c: "baz", extra: "dog" }."#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "for GraphQLInputObject > returns error for an
    /// unknown field"; `coerceInputValue-test.ts` "for GraphQLInputObject > invalid for an unknown
    /// field", "for GraphQLInputObject > returns error for an unknown field"
    #[test]
    fn returns_error_for_an_unknown_field() {
        rejects(
            INT_INPUT,
            r#"{"v":{"foo":123,"unknownField":123}}"#,
            "v",
            "",
            r#"Expected value of type "IntInputObject" not to include unknown field "unknownField", found: { foo: 123, unknownField: 123 }."#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "for GraphQLInputObject > returns error for a
    /// misspelled field (no suggestions)"; `coerceInputValue-test.ts` "for GraphQLInputObject >
    /// returns error for a misspelled field" (the variant without suggestions)
    #[test]
    fn returns_error_for_a_misspelled_field() {
        rejects(
            INT_INPUT,
            r#"{"v":{"foo":123,"bart":123}}"#,
            "v",
            "",
            r#"Expected value of type "IntInputObject" not to include unknown field "bart", found: { foo: 123, bart: 123 }."#,
        );
    }

    /// A `null` value still counts as a provided field.
    #[test]
    fn unknown_field_with_null_value() {
        rejects(
            INT_INPUT,
            r#"{"v":{"foo":123,"unknownField":null}}"#,
            "v",
            "",
            r#"Expected value of type "IntInputObject" not to include unknown field "unknownField", found: { foo: 123, unknownField: null }."#,
        );
    }

    /// spec §2.1.8: field names are case-sensitive.
    #[test]
    fn field_names_are_case_sensitive() {
        rejects(
            TEST_INPUT,
            r#"{"input":{"A":"foo","c":"baz"}}"#,
            "input",
            "",
            r#"Expected value of type "TestInputObject" not to include unknown field "A", found: { A: "foo", c: "baz" }."#,
        );
    }

    #[test]
    fn unknown_field_in_nested_object() {
        rejects(
            NESTED_INPUT,
            r#"{"input":{"na":{"c":"x","extra":1},"nb":"y"}}"#,
            "input",
            " at .na",
            r#"Expected value of type "TestInputObject" not to include unknown field "extra", found: { c: "x", extra: 1 }."#,
        );
    }

    /// A prototype-pollution key never reaches JavaScript subgraphs.
    #[test]
    fn proto_key_is_unknown_field() {
        rejects(
            TEST_INPUT,
            r#"{"input":{"c":"x","__proto__":{"a":1}}}"#,
            "input",
            "",
            r#"Expected value of type "TestInputObject" not to include unknown field "__proto__", found: { c: "x", __proto__: { a: 1 } }."#,
        );
    }
}

/// spec §3.10: each field is coerced by its own type.
mod nested_fields {
    use super::*;

    /// graphql-js `variables-test.ts` "errors on deep nested errors and with many errors". Only the
    /// first error is reported (graphql-js also reports the missing `nb`).
    #[test]
    fn errors_on_deep_nested_errors_and_with_many_errors() {
        rejects(
            NESTED_INPUT,
            r#"{"input":{"na":{"a":"foo"}}}"#,
            "input",
            " at .na",
            r#"Expected value of type "TestInputObject" to include required field "c", found: { a: "foo" }."#,
        );
    }

    #[test]
    fn nested_field_scalar_type_checked() {
        rejects(
            INT_INPUT,
            r#"{"v":{"foo":123,"nested":{"foo":"x"}}}"#,
            "v",
            " at .nested.foo",
            r#"Int cannot represent non-integer value: "x""#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "for GraphQLInputObject > returns an error for an
    /// invalid field"; `coerceInputValue-test.ts` "for GraphQLInputObject > invalid for an invalid
    /// field", "for GraphQLInputObject > returns an error for an invalid field" (`NaN` there; JSON
    /// cannot carry it)
    #[test]
    fn returns_an_error_for_an_invalid_field() {
        rejects(
            INT_INPUT,
            r#"{"v":{"foo":1.5}}"#,
            "v",
            " at .foo",
            "Int cannot represent non-integer value: 1.5",
        );
    }

    /// graphql-js `variables-test.ts` "executes with complex scalar input". Custom scalars are
    /// opaque, so the unknown-field check must not look inside `d`.
    #[test]
    fn executes_with_complex_scalar_input() {
        for d in [r#""ExternalValue""#, r#"{"unknown":1}"#, "[1]"] {
            accepts(TEST_INPUT, &format!(r#"{{"input":{{"c":"foo","d":{d}}}}}"#));
        }
    }
}

mod recursive_types {
    use super::*;

    #[test]
    fn recursive_input_moderately_deep_valid() {
        let mut value = r#"{"name":"x"}"#.to_string();
        for _ in 0..10 {
            value = format!(r#"{{"and":[{value}]}}"#);
        }
        accepts(FILTER, &format!(r#"{{"f":{value}}}"#));
    }

    #[test]
    fn recursive_input_invalid_leaf_path() {
        rejects(
            FILTER,
            r#"{"f":{"and":[{"and":[{"name":1}]}]}}"#,
            "f",
            " at .and[0].and[0].name",
            "String cannot represent a non string value: 1",
        );
    }

    #[test]
    fn recursive_input_null_item() {
        rejects(
            FILTER,
            r#"{"f":{"and":[{"and":[{"name":"a"},null]}]}}"#,
            "f",
            " at .and[0].and[1]",
            r#"Expected value of non-null type "Filter!" not to be null."#,
        );
    }

    #[test]
    fn recursive_self_nested_missing_required() {
        rejects(
            INT_INPUT,
            r#"{"v":{"foo":1,"nested":{"foo":2,"nested":{"bar":3}}}}"#,
            "v",
            " at .nested.nested",
            r#"Expected value of type "IntInputObject" to include required field "foo", found: { bar: 3 }."#,
        );
    }

    /// graphql-js `inspect` prints nested objects to a depth of 2.
    #[test]
    fn found_value_truncated_at_depth_two() {
        rejects(
            INT_INPUT,
            r#"{"v":{"nested":{"nested":{"foo":1}}}}"#,
            "v",
            "",
            r#"Expected value of type "IntInputObject" to include required field "foo", found: { nested: { nested: [Object] } }."#,
        );
    }
}

/// spec §3.10: an input field keeps its list and non-null wrappers.
mod field_wrappers {
    use super::*;

    /// graphql-js `variables-test.ts` "using variables > executes with complex input"
    #[test]
    fn executes_with_complex_input() {
        accepts(TEST_INPUT, r#"{"input":{"a":"foo","b":["bar"],"c":"baz"}}"#);
    }

    /// graphql-js `variables-test.ts` "using variables > properly parses single value to list".
    /// graphql-js coerces `b` to `["bar"]`; the router forwards it as sent.
    #[test]
    fn properly_parses_single_value_to_list() {
        accepts(TEST_INPUT, r#"{"input":{"a":"foo","b":"bar","c":"baz"}}"#);
    }

    #[test]
    fn list_field_nullable_items() {
        accepts(TEST_INPUT, r#"{"input":{"b":["bar",null],"c":"baz"}}"#);
    }

    #[test]
    fn list_field_item_type_checked() {
        rejects(
            TEST_INPUT,
            r#"{"input":{"b":["bar",1],"c":"baz"}}"#,
            "input",
            " at .b[1]",
            "String cannot represent a non string value: 1",
        );
    }

    #[test]
    fn list_field_single_value_type_checked() {
        rejects(
            TEST_INPUT,
            r#"{"input":{"b":1,"c":"baz"}}"#,
            "input",
            " at .b",
            "String cannot represent a non string value: 1",
        );
    }

    #[test]
    fn non_null_item_list_field_single_value() {
        accepts(WRAPPERS, r#"{"v":{"list":1}}"#);
    }

    #[test]
    fn non_null_item_list_field_null_item() {
        rejects(
            WRAPPERS,
            r#"{"v":{"list":[1,null]}}"#,
            "v",
            " at .list[1]",
            r#"Expected value of non-null type "Int!" not to be null."#,
        );
    }

    #[test]
    fn nullable_list_field_null_or_empty() {
        accepts(WRAPPERS, r#"{"v":{"list":null}}"#);
        accepts(WRAPPERS, r#"{"v":{"list":[]}}"#);
    }

    #[test]
    fn list_of_objects_field_required_field_in_item() {
        rejects(
            WRAPPERS,
            r#"{"v":{"nested":[{"b":1},{"a":"x"}]}}"#,
            "v",
            " at .nested[1]",
            r#"Expected value of type "ExampleInputObject" to include required field "b", found: { a: "x" }."#,
        );
    }

    #[test]
    fn list_of_objects_field_item_field_type() {
        rejects(
            WRAPPERS,
            r#"{"v":{"nested":[{"b":1},{"b":"2"}]}}"#,
            "v",
            " at .nested[1].b",
            r#"Int cannot represent non-integer value: "2""#,
        );
    }

    #[test]
    fn list_of_objects_field_null_item() {
        rejects(
            WRAPPERS,
            r#"{"v":{"nested":[{"b":1},null]}}"#,
            "v",
            " at .nested[1]",
            r#"Expected value of non-null type "ExampleInputObject!" not to be null."#,
        );
    }

    #[test]
    fn list_of_objects_field_single_object() {
        accepts(WRAPPERS, r#"{"v":{"nested":{"b":1}}}"#);
    }

    #[test]
    fn nested_object_field_null_for_non_null() {
        rejects(
            WRAPPERS,
            r#"{"v":{"inner":{"b":null}}}"#,
            "v",
            " at .inner.b",
            r#"Expected value of non-null type "Int!" not to be null."#,
        );
    }

    #[test]
    fn non_null_list_field_accepts_empty_null_item_single() {
        for items in ["[]", "[null]", r#""x""#] {
            accepts(NN_LIST_FIELD, &format!(r#"{{"v":{{"items":{items}}}}}"#));
        }
    }

    #[test]
    fn non_null_list_field_rejects_null() {
        rejects(
            NN_LIST_FIELD,
            r#"{"v":{"items":null}}"#,
            "v",
            " at .items",
            r#"Expected value of non-null type "[String]!" not to be null."#,
        );
    }

    #[test]
    fn non_null_list_field_required() {
        rejects(
            NN_LIST_FIELD,
            r#"{"v":{}}"#,
            "v",
            "",
            r#"Expected value of type "NNListField" to include required field "items", found: {}."#,
        );
    }
}

/// spec §3.11: each item of a list of input objects is an input object value.
mod in_lists {
    use super::*;

    #[test]
    fn list_of_input_objects_item_field_invalid() {
        rejects(
            LENGTH_OBJECTS,
            r#"{"v":[{"length":1},{"length":"x"}]}"#,
            "v",
            " at [1].length",
            r#"Int cannot represent non-integer value: "x""#,
        );
    }

    #[test]
    fn list_of_input_objects_non_object_item() {
        rejects(
            LENGTH_OBJECTS,
            r#"{"v":[{"length":1},"x"]}"#,
            "v",
            " at [1]",
            r#"Expected value of type "LengthObject" to be an object, found: "x"."#,
        );
    }

    #[test]
    fn list_of_input_objects_unknown_field_in_item() {
        rejects(
            LENGTH_OBJECTS,
            r#"{"v":[{"length":1},{"len":2}]}"#,
            "v",
            " at [1]",
            r#"Expected value of type "LengthObject" not to include unknown field "len", found: { len: 2 }."#,
        );
    }

    #[test]
    fn unknown_field_in_list_field_item() {
        rejects(
            WRAPPERS,
            r#"{"v":{"nested":[{"b":1},{"b":2,"x":1}]}}"#,
            "v",
            " at .nested[1]",
            r#"Expected value of type "ExampleInputObject" not to include unknown field "x", found: { b: 2, x: 1 }."#,
        );
    }

    #[test]
    fn list_of_input_objects_single_object() {
        accepts(LENGTH_OBJECTS, r#"{"v":{"length":1}}"#);
    }

    #[test]
    fn list_of_input_objects_null_item() {
        accepts(LENGTH_OBJECTS, r#"{"v":[{"length":1},null]}"#);
    }

    /// spec §6.1.2: coercion stops at the first error.
    #[test]
    fn many_invalid_items_one_request_error() {
        let items = vec![r#"{"length":"x"}"#; 100].join(",");
        rejects(
            LENGTH_OBJECTS,
            &format!(r#"{{"v":[{items}]}}"#),
            "v",
            " at [0].length",
            r#"Int cannot represent non-integer value: "x""#,
        );
    }
}

/// The first error follows the type's field order, not the order of the JSON keys.
mod error_order {
    use super::*;

    /// graphql-js `validateInputValue-test.ts` "for GraphQLInputObject > returns multiple errors
    /// for multiple invalid fields"; `coerceInputValue-test.ts` "invalid for multiple invalid
    /// fields", "for GraphQLInputObject > returns multiple errors for multiple invalid fields".
    /// Only the first error is reported (graphql-js also reports `.bar`).
    #[test]
    fn returns_multiple_errors_for_multiple_invalid_fields() {
        rejects(
            INT_INPUT,
            r#"{"v":{"foo":"abc","bar":"def"}}"#,
            "v",
            " at .foo",
            r#"Int cannot represent non-integer value: "abc""#,
        );
    }

    /// spec §2.10.8
    #[test]
    fn first_error_independent_of_key_order() {
        rejects(
            INT_INPUT,
            r#"{"v":{"bar":"def","foo":"abc"}}"#,
            "v",
            " at .foo",
            r#"Int cannot represent non-integer value: "abc""#,
        );
    }

    /// graphql-js checks the defined fields before looking for unknown keys.
    #[test]
    fn defined_field_errors_before_unknown_keys() {
        rejects(
            INT_INPUT,
            r#"{"v":{"extra":1,"foo":"abc"}}"#,
            "v",
            " at .foo",
            r#"Int cannot represent non-integer value: "abc""#,
        );
    }

    /// spec §2.1.8 (graphql-js also reports the unknown `B`)
    #[test]
    fn missing_required_reported_before_unknown_key() {
        rejects(
            EXAMPLE_INPUT,
            r#"{"var":{"B":123}}"#,
            "var",
            "",
            r#"Expected value of type "ExampleInputObject" to include required field "b", found: { B: 123 }."#,
        );
    }

    /// spec §2.10.8
    #[test]
    fn key_order_does_not_matter() {
        accepts(TEST_INPUT, r#"{"input":{"c":"baz","b":["bar"],"a":"foo"}}"#);
    }
}

/// The spec's §3.10 example table, for the rows that use JSON variables.
mod spec_examples {
    use super::*;

    #[test]
    fn spec_3_10_all_fields() {
        accepts(EXAMPLE_INPUT, r#"{"var":{"a":"abc","b":123}}"#);
    }

    #[test]
    fn spec_3_10_explicit_null_kept() {
        accepts(EXAMPLE_INPUT, r#"{"var":{"a":null,"b":123}}"#);
    }

    #[test]
    fn spec_3_10_absent_field_not_injected() {
        accepts(EXAMPLE_INPUT, r#"{"var":{"b":123}}"#);
    }

    #[test]
    fn spec_3_10_string_not_object() {
        rejects(
            EXAMPLE_INPUT,
            r#"{"var":"abc123"}"#,
            "var",
            "",
            r#"Expected value of type "ExampleInputObject" to be an object, found: "abc123"."#,
        );
    }

    #[test]
    fn spec_3_10_wrong_field_type() {
        rejects(
            EXAMPLE_INPUT,
            r#"{"var":{"a":"abc","b":"123"}}"#,
            "var",
            " at .b",
            r#"Int cannot represent non-integer value: "123""#,
        );
    }

    #[test]
    fn spec_3_10_missing_required_field() {
        rejects(
            EXAMPLE_INPUT,
            r#"{"var":{"a":"abc"}}"#,
            "var",
            "",
            r#"Expected value of type "ExampleInputObject" to include required field "b", found: { a: "abc" }."#,
        );
    }

    #[test]
    fn spec_3_10_null_for_non_null_field() {
        rejects(
            EXAMPLE_INPUT,
            r#"{"var":{"a":"abc","b":null}}"#,
            "var",
            " at .b",
            r#"Expected value of non-null type "Int!" not to be null."#,
        );
    }

    #[test]
    fn spec_3_10_unexpected_field() {
        rejects(
            EXAMPLE_INPUT,
            r#"{"var":{"b":123,"c":"xyz"}}"#,
            "var",
            "",
            r#"Expected value of type "ExampleInputObject" not to include unknown field "c", found: { b: 123, c: "xyz" }."#,
        );
    }
}

/// Variables used inside an input object literal are coerced by their own type.
mod variables_in_literals {
    use super::*;

    const IN_LITERAL: &str =
        r#"query q($input: String) { fieldWithObjectInput(input: { a: $input, c: "baz" }) }"#;
    const NON_NULL_IN_LITERAL: &str = "query ($var: Int!) { exampleInput(input: {b: $var}) }";

    /// graphql-js `variables-test.ts` "preserves explicit null variables within input object
    /// literals"
    #[test]
    fn preserves_explicit_null_variables_within_input_object_literals() {
        accepts(IN_LITERAL, r#"{"input":null}"#);
    }

    /// spec §3.10
    #[test]
    fn absent_variable_within_input_object_literal() {
        assert_coerced(IN_LITERAL, "{}", "{}");
    }

    /// spec §3.10
    #[test]
    fn spec_3_10_non_null_variable_in_literal() {
        accepts(NON_NULL_IN_LITERAL, r#"{"var":123}"#);
    }

    /// spec §3.10
    #[test]
    fn spec_3_10_non_null_variable_absent() {
        assert_rejected(
            NON_NULL_IN_LITERAL,
            "{}",
            r#"Variable "$var" has invalid value: Expected a value of non-null type "Int!" to be provided."#,
        );
    }

    /// spec §3.10
    #[test]
    fn spec_3_10_non_null_variable_null() {
        rejects(
            NON_NULL_IN_LITERAL,
            r#"{"var":null}"#,
            "var",
            "",
            r#"Expected value of non-null type "Int!" not to be null."#,
        );
    }

    /// spec §3.10 and §6.4.1: the variable is nullable, so `null` passes coercion; the
    /// subgraph raises the error for the non-null field.
    #[test]
    fn spec_3_10_defaulted_variable_null_forwarded() {
        accepts(
            "query ($var: Int = 1) { exampleInput(input: {b: $var}) }",
            r#"{"var":null}"#,
        );
    }

    /// graphql-js `validateInputValue-test.ts` "allows missing variables in an optional field". The
    /// field has a default, so an absent variable is allowed.
    #[test]
    fn allows_missing_variables_in_an_optional_field() {
        assert_coerced(
            "query ($x: Int) { defaultedRequired(input: {x: $x}) }",
            "{}",
            "{}",
        );
    }
}

/// spec §6.1.2: an input object default is coerced by the variable type when it is used.
mod defaults {
    use super::*;

    /// graphql-js `variables-test.ts` "uses default value when not provided"
    #[test]
    fn uses_default_value_when_not_provided() {
        assert_coerced(
            r#"query ($input: TestInputObject = {a: "foo", b: ["bar"], c: "baz"}) { fieldWithObjectInput(input: $input) }"#,
            "{}",
            r#"{"input":{"a":"foo","b":["bar"],"c":"baz"}}"#,
        );
    }

    /// graphql-js `variables-test.ts` "reports invalid default values with variable definition
    /// locations"
    #[test]
    fn reports_invalid_default_values_is_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($input: String = 123) { fieldWithNullableStringInput(input: $input) }",
            "ValuesOfCorrectType",
        );
    }

    /// graphql-js `variables-test.ts` "hides suggestions for invalid default values when specified"
    #[test]
    fn unknown_field_in_default_is_rejected_by_validation() {
        assert_rejected_by_validation(
            r#"query ($input: TestInputObject = { c: "ok", aa: "x" }) { fieldWithObjectInput(input: $input) }"#,
            "ValuesOfCorrectType",
        );
    }

    #[test]
    fn list_default_for_input_object_is_rejected_by_validation() {
        assert_rejected_by_validation(
            "query ($input: TestInputObject = []) { fieldWithObjectInput(input: $input) }",
            "ValuesOfCorrectType",
        );
    }
}
