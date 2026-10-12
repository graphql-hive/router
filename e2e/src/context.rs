use std::sync::{Arc, Mutex};

use graphql_tools::parser::{
    parse_query,
    query::{Definition, OperationDefinition, Selection, SelectionSet, Value as QueryValue},
};
use serde_json::{json, Value};

use crate::testkit::{ClientResponseExt, TestRouter};

fn render(
    selection: &SelectionSet<'_, String>,
    variables: &Value,
    representation: &Value,
) -> Value {
    let mut result = serde_json::Map::new();
    for item in &selection.items {
        match item {
            Selection::InlineFragment(fragment) => {
                result.extend(
                    render(&fragment.selection_set, variables, representation)
                        .as_object()
                        .unwrap()
                        .clone(),
                );
            }
            Selection::Field(field) => {
                let value = if !field.selection_set.items.is_empty() {
                    render(&field.selection_set, variables, representation)
                } else if field.name == "base" {
                    json!(10)
                } else if field.arguments.is_empty() {
                    representation[&field.name].clone()
                } else {
                    let values: Vec<_> = field
                        .arguments
                        .iter()
                        .map(|(_, value)| {
                            let QueryValue::Variable(name) = value else {
                                panic!("context variable");
                            };
                            variables[name].clone()
                        })
                        .collect();
                    json!(values
                        .iter()
                        .map(|value| match value.as_str() {
                            Some("USD") => 1,
                            Some("EUR") => 2,
                            Some("outer") => 10,
                            _ => 0,
                        })
                        .sum::<i32>())
                };
                result.insert(field.alias.as_ref().unwrap_or(&field.name).clone(), value);
            }
            Selection::FragmentSpread(_) => panic!("unexpected output fragment"),
        }
    }
    Value::Object(result)
}

async fn run_case(
    fixture: &str,
    query: &str,
    root_data: Value,
    expected: Value,
    requests: usize,
) -> Vec<Value> {
    let mut server = mockito::Server::new_async().await;
    let host = server.host_with_port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let calls = log.clone();
    let mock = server
        .mock("POST", mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body_from_request(move |request| {
            let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
            if !body["query"].as_str().unwrap().contains("_entities") {
                return json!({"data": root_data}).to_string().into_bytes();
            }
            calls.lock().unwrap().push(body.clone());
            let document = parse_query::<String>(body["query"].as_str().unwrap()).unwrap();
            let Definition::Operation(OperationDefinition::Query(operation)) =
                &document.definitions[0]
            else {
                panic!("entity query");
            };
            let mut data = serde_json::Map::new();
            for item in &operation.selection_set.items {
                let Selection::Field(field) = item else {
                    panic!("entities field");
                };
                let QueryValue::Variable(name) = &field.arguments[0].1 else {
                    panic!("representations variable");
                };
                let entities: Vec<_> = body["variables"][name]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|representation| {
                        render(&field.selection_set, &body["variables"], representation)
                    })
                    .collect();
                data.insert(
                    field.alias.as_ref().unwrap_or(&field.name).clone(),
                    json!(entities),
                );
            }
            json!({"data": data}).to_string().into_bytes()
        })
        .expect(requests + 1)
        .create_async()
        .await;
    let path = format!(
        "{}/../bin/router/fixture/tests/{fixture}.supergraph.graphql",
        env!("CARGO_MANIFEST_DIR")
    );
    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
        supergraph:
          source: file
          path: {path}
        override_subgraph_urls:
          subgraphs:
            a:
              url: "http://{host}/a"
            b:
              url: "http://{host}/b"
    "#
        ))
        .build()
        .start()
        .await;
    let response = router.send_graphql_request(query, None, None).await;
    let response: Value = serde_json::from_str(&response.json_body_string_pretty().await).unwrap();
    assert_eq!(response, json!({"data": expected}), "{query}");
    mock.assert_async().await;
    let result = log.lock().unwrap().clone();
    assert_eq!(result.len(), requests, "{query}");
    result
}

#[ntex::test]
async fn context_groups_lists_nulls_and_deduplication() {
    for (currencies, expected, groups) in [
        (vec![json!("USD"), json!("USD")], vec![1, 1], 1),
        (
            vec![json!("USD"), json!("EUR"), Value::Null],
            vec![1, 2, 0],
            3,
        ),
    ] {
        let data: Vec<_> = currencies.iter().map(|currency| json!({"__typename":"User", "currency":currency,
            "txs":[{"__typename":"Transaction", "id":"same"}, {"__typename":"Transaction", "id":"same"}]})).collect();
        let expected: Vec<_> = expected
            .iter()
            .map(|amount| json!({"txs":[{"amount":amount}, {"amount":amount}]}))
            .collect();
        let calls = run_case(
            "context-basic",
            "{ users { txs { amount } } }",
            json!({"users":data}),
            json!({"users":expected}),
            groups,
        )
        .await;
        for call in calls {
            assert_eq!(
                call["variables"]["representations"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}

#[ntex::test]
async fn context_nearest_nested_abstract_multiple_and_requires() {
    let tx = json!({"__typename":"Transaction", "id":"1"});
    let user = json!({"__typename":"User", "currency":"USD", "txs":[tx.clone()]});
    for (query, data, expected, requests) in [
        (
            "{ users { child { txs { amount } } } }",
            json!({"users":[{"__typename":"User", "currency":"EUR", "child":user.clone()}]}),
            json!({"users":[{"child":{"txs":[{"amount":1}]}}]}),
            1,
        ),
        (
            "{ users { wrapper { txs { amount } } } }",
            json!({"users":[{"__typename":"User", "currency":"USD", "wrapper":{"txs":[tx.clone()]}}]}),
            json!({"users":[{"wrapper":{"txs":[{"amount":1}]}}]}),
            1,
        ),
        (
            "{ account { txs { fromInterface } } }",
            json!({"account":user.clone()}),
            json!({"account":{"txs":[{"fromInterface":1}]}}),
            1,
        ),
        (
            "{ viewer { ... on Other { txs { fromUnion } } } }",
            json!({"viewer":{"__typename":"Other", "currency":"EUR", "txs":[tx.clone()]}}),
            json!({"viewer":{"txs":[{"fromUnion":2}]}}),
            1,
        ),
        (
            "{ outer { user { txs { multiple } } } }",
            json!({"outer":{"__typename":"Outer", "code":"outer", "user":user.clone()}}),
            json!({"outer":{"user":{"txs":[{"multiple":11}]}}}),
            1,
        ),
        (
            "{ users { txs { required } } }",
            json!({"users":[user.clone()]}),
            json!({"users":[{"txs":[{"required":1}]}]}),
            2,
        ),
        (
            "{ renamed: users { currency: id txs { amount } } }",
            json!({"renamed":[{"__typename":"User", "currency":"user-id", "_internal_qp_alias_0":"USD", "txs":[tx.clone()]}]}),
            json!({"renamed":[{"currency":"user-id", "txs":[{"amount":1}]}]}),
            1,
        ),
        (
            "{ users { txs { amount @skip(if: true) } } }",
            json!({"users":[{"txs":[tx.clone()]}]}),
            json!({"users":[{"txs":[{}]}]}),
            0,
        ),
    ] {
        run_case("context-basic", query, data, expected, requests).await;
    }
}

#[ntex::test]
async fn context_cross_subgraph_merges_into_ancestor_fetch() {
    let calls = run_case(
        "context-cross",
        "{ user { lastTransaction { amountInUserCurrency } } }",
        json!({"user":{"__typename":"User", "id":"1", "userCurrency":{"isoCode":"EUR"}}}),
        json!({"user":{"lastTransaction":{"amountInUserCurrency":2}}}),
        1,
    )
    .await;
    assert_eq!(calls[0]["variables"]["__context_0"], "EUR");
    assert!(calls[0]["query"].as_str().unwrap().contains("...on User"));
}

#[ntex::test]
async fn context_batch_aliases_have_different_group_counts() {
    let tx = json!({"__typename":"Transaction", "id":"same"});
    for second in [
        json!([{"__typename":"User", "currency":"USD", "txs":[tx.clone()]}]),
        json!([]),
    ] {
        let expected_second = if second.as_array().unwrap().is_empty() {
            json!([])
        } else {
            json!([{"txs":[{"amount":1}]}])
        };
        let calls = run_case("context-basic", "{ users { txs { amount } } secondUsers { txs { amount } } }",
            json!({"users":[{"__typename":"User", "currency":"USD", "txs":[tx.clone()]}, {"__typename":"User", "currency":"EUR", "txs":[tx.clone()]}], "secondUsers":second}),
            json!({"users":[{"txs":[{"amount":1}]}, {"txs":[{"amount":2}]}], "secondUsers":expected_second}), 2).await;
        assert!(calls.iter().any(|call| call["variables"]
            .as_object()
            .unwrap()
            .iter()
            .any(|(name, value)| name.starts_with("__batch_reps_") && value == &json!([]))));
    }
}
