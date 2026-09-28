//! Runs an operation and its JSON variables through the same steps the router pipeline uses
//! before coercion (parse, normalize, partition), then calls the real `collect_variables`.

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use graphql_tools::validation::rules::default_rules_validation_plan;
use graphql_tools::validation::validate::validate;
use sonic_rs::Value;

use crate::executor::introspection::partition::partition_operation;
use crate::executor::introspection::schema::{SchemaMetadata, SchemaWithMetadata};
use crate::executor::variables::{collect_variables, VariableCoercionError};
use crate::query_planner::ast::normalization::normalize_operation;
use crate::query_planner::consumer_schema::ConsumerSchema;
use crate::query_planner::state::supergraph_state::SupergraphState;
use crate::query_planner::utils::parsing::{parse_schema, safe_parse_operation};

struct TestSchema {
    supergraph_state: SupergraphState,
    consumer_schema: ConsumerSchema,
    metadata: SchemaMetadata,
}

static SCHEMA: LazyLock<TestSchema> = LazyLock::new(|| {
    let supergraph = parse_schema(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixture/variables_coercion/supergraph.graphql"
    )));
    let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
    let metadata = consumer_schema.schema_metadata();

    TestSchema {
        supergraph_state: SupergraphState::new(&supergraph),
        consumer_schema,
        metadata,
    }
});

/// What coercion produced for a request.
#[derive(Debug, PartialEq)]
pub(super) enum Outcome {
    /// The coerced variables. A variable with no entry is absent (not `null`).
    Coerced(BTreeMap<String, Value>),
    /// The request error messages.
    Rejected(Vec<String>),
}

/// Coerces `variables` (a JSON object) for `operation`, without running validation first.
pub(super) fn coerce(operation: &str, variables: &str) -> Outcome {
    let document = safe_parse_operation(operation)
        .unwrap_or_else(|err| panic!("failed to parse operation: {err}\n{operation}"));
    let normalized = normalize_operation(&SCHEMA.supergraph_state, &document, None)
        .unwrap_or_else(|err| panic!("failed to normalize operation: {err}\n{operation}"));
    let operation_for_plan = partition_operation(normalized.operation).downstream_operation;

    let mut variables: HashMap<String, Value> = sonic_rs::from_str(variables)
        .unwrap_or_else(|err| panic!("variables must be a JSON object: {err}\n{variables}"));

    match collect_variables(&operation_for_plan, &mut variables, &SCHEMA.metadata) {
        Ok(coerced) => Outcome::Coerced(coerced.unwrap_or_default().into_iter().collect()),
        Err(err) => Outcome::Rejected(messages(&err)),
    }
}

/// The request error messages for a coercion error. The only place that knows the error type.
fn messages(err: &VariableCoercionError) -> Vec<String> {
    vec![err.to_string()]
}

/// Asserts that coercion accepts `variables` and produces `expected` (a JSON object).
#[track_caller]
pub(super) fn assert_coerced(operation: &str, variables: &str, expected: &str) {
    let expected: BTreeMap<String, Value> = sonic_rs::from_str(expected)
        .unwrap_or_else(|err| panic!("expected must be a JSON object: {err}\n{expected}"));
    assert_eq!(
        coerce(operation, variables),
        Outcome::Coerced(expected),
        "\noperation: {operation}\nvariables: {variables}"
    );
}

/// Asserts that coercion accepts `variables` and that the coerced variables serialize to
/// exactly `expected`, the JSON text sent to subgraphs (keys sorted). Unlike `assert_coerced`,
/// this catches numbers whose digits change on the way through.
#[track_caller]
pub(super) fn assert_forwarded(operation: &str, variables: &str, expected: &str) {
    match coerce(operation, variables) {
        Outcome::Coerced(coerced) => assert_eq!(
            sonic_rs::to_string(&coerced).unwrap(),
            expected,
            "\noperation: {operation}\nvariables: {variables}"
        ),
        rejected => panic!(
            "expected the variables to be accepted, got {rejected:?}\noperation: {operation}\nvariables: {variables}"
        ),
    }
}

/// Asserts that the router's JSON parser rejects `variables`, so the request fails before
/// coercion runs.
#[track_caller]
pub(super) fn assert_invalid_json(variables: &str) {
    let parsed = sonic_rs::from_str::<HashMap<String, Value>>(variables);
    assert!(
        parsed.is_err(),
        "expected invalid JSON, parsed {parsed:?}\nvariables: {variables}"
    );
}

/// Asserts that coercion rejects `variables` with exactly one request error, `expected`.
#[track_caller]
pub(super) fn assert_rejected(operation: &str, variables: &str, expected: &str) {
    assert_eq!(
        coerce(operation, variables),
        Outcome::Rejected(vec![expected.to_string()]),
        "\noperation: {operation}\nvariables: {variables}"
    );
}

/// Asserts that the router's validation rejects `operation` with an error from `rule`
/// (a graphql-tools error code, e.g. `ValuesOfCorrectType`).
#[track_caller]
pub(super) fn assert_rejected_by_validation(operation: &str, rule: &str) {
    let document = safe_parse_operation(operation)
        .unwrap_or_else(|err| panic!("failed to parse operation: {err}\n{operation}"));
    let errors = validate(
        &SCHEMA.consumer_schema.document,
        &document,
        &default_rules_validation_plan(),
    );
    assert!(
        errors.iter().any(|error| error.error_code == rule),
        "expected a {rule} validation error\noperation: {operation}\nerrors: {errors:#?}"
    );
}
