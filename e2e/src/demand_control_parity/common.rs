use std::path::PathBuf;

use serde_json::json as sjson;
pub(super) use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value as SonicValue};

pub(super) use crate::testkit::{
    mock_subgraphs::mock_subgraphs, ClientResponseExt, TestRouter, TestSubgraphs,
};

/// Reasonable default max that should not be exceeded by any of these
/// tests; individual cases lower it to assert rejection paths.
pub(super) const MAX_COST: u64 = 10_000_000;

pub(super) fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("demand_control")
        .join(name)
}

pub(super) fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("must read fixture {name}: {e}"))
}

/// Mirror of upstream `TestSetupParameters`: a fixture identifies a
/// schema, a query and the canned subgraph responses to feed to the
/// mock harness.
#[derive(Clone)]
pub(super) struct Fixture {
    pub(super) schema_file: &'static str,
    pub(super) query_file: &'static str,
    pub(super) subgraphs: serde_json::Value,
}

pub(super) fn basic_fragments() -> Fixture {
    Fixture {
        schema_file: "basic_supergraph_schema.graphql",
        query_file: "basic_fragments_query.graphql",
        subgraphs: sjson!({
            "products": {
                "query": {
                    "interfaceInstance1": {
                        "__typename": "SecondObjectType",
                        "field1": null,
                        "field2": "hello"
                    },
                    "someUnion": {
                        "__typename": "FirstObjectType",
                        "innerList": []
                    }
                }
            }
        }),
    }
}

pub(super) fn basic_mutation() -> Fixture {
    Fixture {
        schema_file: "basic_supergraph_schema.graphql",
        query_file: "basic_mutation.graphql",
        subgraphs: sjson!({
            "products": {
                "mutation": { "doSomething": 6 }
            }
        }),
    }
}

pub(super) fn federated_ships_required() -> Fixture {
    Fixture {
        schema_file: "federated_ships_schema.graphql",
        query_file: "federated_ships_required_query.graphql",
        subgraphs: sjson!({
            "vehicles": {
                "query": {
                    "ships": [
                        {"__typename": "Ship", "id": 1, "name": "Ship1", "owner": {"__typename": "User", "licenseNumber": 10}},
                        {"__typename": "Ship", "id": 2, "name": "Ship2", "owner": {"__typename": "User", "licenseNumber": 11}},
                        {"__typename": "Ship", "id": 3, "name": "Ship3", "owner": {"__typename": "User", "licenseNumber": 12}},
                    ]
                },
                "entities": [
                    {"__typename": "Ship", "id": 1, "owner": {"addresses": [{"zipCode": 18263}]}, "registrationFee": 129.2},
                    {"__typename": "Ship", "id": 2, "owner": {"addresses": [{"zipCode": 61027}]}, "registrationFee": 14.0},
                    {"__typename": "Ship", "id": 3, "owner": {"addresses": [{"zipCode": 86204}]}, "registrationFee": 97.15},
                ]
            },
            "users": {
                "entities": [
                    {"__typename": "User", "licenseNumber": 10, "addresses": [{"zipCode": 18263}]},
                    {"__typename": "User", "licenseNumber": 11, "addresses": [{"zipCode": 61027}]},
                    {"__typename": "User", "licenseNumber": 12, "addresses": [{"zipCode": 86204}]},
                ]
            }
        }),
    }
}

pub(super) fn federated_ships_fragment() -> Fixture {
    Fixture {
        schema_file: "federated_ships_schema.graphql",
        query_file: "federated_ships_fragment_query.graphql",
        subgraphs: sjson!({
            "vehicles": {
                "query": {
                    "ships": [
                        {"__typename": "Ship", "id": 1, "name": "Ship1", "owner": {"__typename": "User", "licenseNumber": 100}},
                        {"__typename": "Ship", "id": 2, "name": "Ship2", "owner": {"__typename": "User", "licenseNumber": 110}},
                        {"__typename": "Ship", "id": 3, "name": "Ship3", "owner": {"__typename": "User", "licenseNumber": 120}},
                        {"__typename": "Ship", "id": 4, "name": "Ship4", "owner": {"__typename": "User", "licenseNumber": 120}},
                        {"__typename": "Ship", "id": 5, "name": "Ship5", "owner": {"__typename": "User", "licenseNumber": 120}},
                    ]
                }
            },
            "users": {
                "query": {
                    "users": [
                        {"__typename": "User", "name": "User10", "licenseNumber": 10},
                        {"__typename": "User", "name": "User11", "licenseNumber": 11},
                    ]
                },
                "entities": [
                    {"__typename": "User", "name": "User100", "licenseNumber": 100},
                    {"__typename": "User", "name": "User110", "licenseNumber": 110},
                    {"__typename": "User", "name": "User120", "licenseNumber": 120},
                ]
            }
        }),
    }
}

pub(super) fn custom_costs() -> Fixture {
    Fixture {
        schema_file: "custom_cost_schema.graphql",
        query_file: "custom_cost_query.graphql",
        subgraphs: sjson!({
            "subgraphWithCost": {
                "query": {
                    "fieldWithCost": 2,
                    "argWithCost": 30,
                    "enumWithCost": "A",
                    "inputWithCost": 5,
                    "scalarWithCost": 6172364,
                    "objectWithCost": { "id": 9 }
                }
            },
            "subgraphWithListSize": {
                "query": {
                    "fieldWithListSize": ["hello", "world", "and", "nearby", "planets"],
                    "fieldWithDynamicListSize": { "items": [{"id": 7}, {"id": 9}] }
                }
            }
        }),
    }
}

/// Per-fixture expected `subgraph_call_count` values when the request
/// is accepted and fully executed (the reference `subgraph_call_count`
/// values for `requests_within_max_are_accepted`).
pub(super) fn expected_call_counts(fixture: &Fixture) -> &'static [(&'static str, usize)] {
    match fixture.query_file {
        "basic_fragments_query.graphql" => &[("products", 1)],
        "basic_mutation.graphql" => &[("products", 1)],
        "federated_ships_required_query.graphql" => &[("users", 1), ("vehicles", 2)],
        "federated_ships_fragment_query.graphql" => &[("users", 2), ("vehicles", 1)],
        "custom_cost_query.graphql" => &[("subgraphWithCost", 1), ("subgraphWithListSize", 1)],
        other => panic!("expected_call_counts: unknown fixture {other}"),
    }
}

/// Stand up subgraphs (with the canned mock responses) and a router
/// pointing at them with the given inline `demand_control` YAML block,
/// run the fixture query and return the JSON response body.
/// Outcome of running a fixture: the GraphQL response body and the
/// per-subgraph request counts collected from the mock harness. Mirrors
/// the data the reference snapshots assert on (response body +
/// per-subgraph call counts).
pub(super) struct RunOutcome {
    pub(super) json: SonicValue,
    pub(super) call_counts: std::collections::BTreeMap<String, usize>,
}

pub(super) async fn run_fixture(fixture: &Fixture, demand_control_yaml: &str) -> RunOutcome {
    let schema_path = fixture_path(fixture.schema_file);
    let query = read_fixture(fixture.query_file);

    let subgraphs = TestSubgraphs::builder()
        .with_on_request(mock_subgraphs(fixture.subgraphs.clone()))
        .build()
        .start()
        .await;

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
            supergraph:
                source: file
                path: "{schema}"
            demand_control:
{dc}
            "#,
            schema = schema_path.to_string_lossy(),
            dc = indent_block(demand_control_yaml, "                    "),
        ))
        .build()
        .start()
        .await;

    let res = router.send_graphql_request(&query, None, None).await;
    let json = res.json_body().await;

    let mut call_counts = std::collections::BTreeMap::new();
    if let Some(map) = fixture.subgraphs.as_object() {
        for name in map.keys() {
            let count = subgraphs
                .get_requests_log(name)
                .map(|log| log.len())
                .unwrap_or(0);
            call_counts.insert(name.clone(), count);
        }
    }

    RunOutcome { json, call_counts }
}

fn indent_block(block: &str, prefix: &str) -> String {
    block
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Asserts the response was accepted (no top-level GraphQL errors and
/// `data` is present and non-null).
pub(super) fn assert_accepted(json: &SonicValue, label: &str) {
    let errors = json.get("errors");
    assert!(
        errors.is_none()
            || errors
                .unwrap()
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(true),
        "[{label}] expected no top-level errors but got: {json}"
    );
    assert!(
        json.get("data").is_some_and(|d| !d.is_null()),
        "[{label}] expected data to be present and non-null: {json}"
    );
}

pub(super) fn assert_rejected_estimated(json: &SonicValue, label: &str) {
    let code = json["errors"][0]["extensions"]["code"].as_str();
    assert_eq!(
        code,
        Some("COST_ESTIMATED_TOO_EXPENSIVE"),
        "[{label}] expected COST_ESTIMATED_TOO_EXPENSIVE rejection: {json}"
    );
}

/// Reads a numeric value from `json` at the given path, accepting any of
/// the JSON number forms (the reference emits `12.0`, our router may emit `12`).
fn cost_num(json: &SonicValue, path: &[&str]) -> Option<f64> {
    let mut cur = json;
    for p in path {
        let next = cur.get(p)?;
        cur = next;
    }
    cur.as_f64()
        .or_else(|| cur.as_u64().map(|n| n as f64))
        .or_else(|| cur.as_i64().map(|n| n as f64))
}

/// Asserts `extensions.cost.estimated` and `extensions.cost.actual` match
/// the reference snapshot values exactly.
pub(super) fn assert_cost(
    json: &SonicValue,
    label: &str,
    expected_estimated: f64,
    expected_actual: f64,
) {
    let est = cost_num(json, &["extensions", "cost", "estimated"])
        .unwrap_or_else(|| panic!("[{label}] missing extensions.cost.estimated: {json}"));
    let act = cost_num(json, &["extensions", "cost", "actual"])
        .unwrap_or_else(|| panic!("[{label}] missing extensions.cost.actual: {json}"));
    assert_eq!(
        est, expected_estimated,
        "[{label}] estimated cost drift (expected={expected_estimated}, ours={est}): {json}"
    );
    assert_eq!(
        act, expected_actual,
        "[{label}] actual cost drift (expected={expected_actual}, ours={act}): {json}"
    );
}

/// Asserts the per-subgraph request counts captured by the mock harness
/// match the expected per-subgraph counts captured for the fixtures.
/// snapshot value. This catches query-plan regressions (missing
/// or duplicated fetches) that pure cost-number assertions would miss.
pub(super) fn assert_call_counts(outcome: &RunOutcome, label: &str, expected: &[(&str, usize)]) {
    for (subgraph, count) in expected {
        let actual = outcome.call_counts.get(*subgraph).copied().unwrap_or(0);
        assert_eq!(
            actual, *count,
            "[{label}] subgraph_call_count.{subgraph} drift \
             (expected={count}, ours={actual}); full counts: {:?}",
            outcome.call_counts
        );
    }
}

/// Macro: emit one `#[ntex::test]` per fixture, calling the given async
/// `run_case(fixture)` body. Mirrors the reference's
/// `#[rstest::rstest] #[values(..)]` per-fixture parametrization.
macro_rules! parametric_per_fixture {
    () => {
        #[ntex::test]
        async fn basic_fragments() {
            run_case(super::super::common::basic_fragments()).await
        }
        #[ntex::test]
        async fn basic_mutation() {
            run_case(super::super::common::basic_mutation()).await
        }
        #[ntex::test]
        async fn federated_ships_required() {
            run_case(super::super::common::federated_ships_required()).await
        }
        #[ntex::test]
        async fn federated_ships_fragment() {
            run_case(super::super::common::federated_ships_fragment()).await
        }
        #[ntex::test]
        async fn custom_costs() {
            run_case(super::super::common::custom_costs()).await
        }
    };
}
pub(super) use parametric_per_fixture;
