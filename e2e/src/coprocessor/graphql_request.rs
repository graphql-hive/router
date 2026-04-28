use sonic_rs::json;
use sonic_rs::JsonValueTrait;

use crate::testkit::{coprocessor::TestCoprocessor, ClientResponseExt, TestRouter, TestSubgraphs};

#[ntex::test]
/// This test checks that graphql.request sends GraphQL inputs inside a nested body object,
/// and that only selected fields are present there.
async fn uses_nested_body_with_selected_fields() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.request", |payload| {
            // Only `query` and `extensions` should be present
            if payload.pointer(&["body", "extensions"]).is_none() {
                return false;
            }
            if payload.pointer(&["body", "query"]).is_none() {
                return false;
            }

            // `variables` and `operation_name` should be absent
            payload.pointer(&["body", "variables"]).is_none()
                && payload.pointer(&["body", "operation_name"]).is_none()
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"version": 1, "control": "continue"}).to_string())
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    graphql:
                      request:
                        include:
                          body: [query, extensions]
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .serv()
        .post(router.graphql_path())
        .content_type("application/json")
        .send_json(&json!({
          "query": "{ topProducts(first:1) { name } }",
          "extensions": { "persisted": true }
        }))
        .await
        .expect("failed to send graphql request");

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table"
              }
            ]
          }
        }
        "#);
    assert!(
        response.status().is_success(),
        "router should return successful response"
    );

    request_stage_mock.assert_async().await;
}

#[ntex::test]
async fn does_not_send_body_when_include_body_false() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.request", |payload| payload.get("body").is_none())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"version": 1, "control": "continue"}).to_string())
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    graphql:
                      request:
                        include:
                          body: false
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first:1) { name } }", None, None)
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table"
              }
            ]
          }
        }
        "#);
    assert!(
        response.status().is_success(),
        "router should return successful response"
    );

    request_stage_mock.assert_async().await;
}

#[ntex::test]
/// This test checks that graphql.request applies body mutations returned by coprocessor,
/// even when include.body is false on the outbound request payload.
async fn applies_body_mutation_when_include_body_false() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.request", |payload| payload.get("body").is_none())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "version": 1,
                "control": "continue",
                "body": {
                  "query": "{ topProducts(first:1) { name } }",
                  "extensions":{
                    "coprocessor": true
                  }
                }
              }
            )
            .to_string(),
        )
        .expect(1)
        .create();

    let analysis_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.analysis", |payload| {
            payload
                .pointer(&["body", "query"])
                .and_then(|query| query.as_str())
                .and_then(|query| {
                    let yes_top_products = query.contains("topProducts");
                    let yes_name = query.contains("name");
                    let no_price = !query.contains("price");

                    (yes_top_products && yes_name && no_price).into()
                })
                == Some(true)
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"version": 1, "control": "continue"}).to_string())
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    graphql:
                      request:
                        include:
                          body: false
                      analysis:
                        include:
                          body: [query, extensions]
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first:1) { name price } }", None, None)
        .await;

    // It's up to the coprocessor to return valid response.
    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table"
              }
            ]
          }
        }
        "#);

    assert!(
        response.status().is_success(),
        "router should accept graphql.request body mutation and return successful response"
    );

    request_stage_mock.assert_async().await;
    analysis_stage_mock.assert_async().await;
}

#[ntex::test]
/// This test checks that graphql.request applies body mutations returned by coprocessor,
/// even when `body` is a stringified JSON value,
/// and even when include.body is false on the outbound request payload.
async fn applies_body_as_string_mutation_when_include_body_false() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.request", |payload| payload.get("body").is_none())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "version": 1,
                "control": "continue",
                "body": json!({
                  "query": "{ topProducts(first:1) { name } }",
                  "extensions":{
                    "coprocessor": true
                  }
                }).to_string(),
              }
            )
            .to_string(),
        )
        .expect(1)
        .create();

    let analysis_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.analysis", |payload| {
            if payload
                .pointer(&["body", "extensions", "coprocessor"])
                .and_then(|val| val.as_bool())
                != Some(true)
            {
                return false;
            }

            payload
                .pointer(&["body", "query"])
                .and_then(|query| query.as_str())
                .and_then(|query| {
                    let yes_top_products = query.contains("topProducts");
                    let yes_name = query.contains("name");
                    let no_price = !query.contains("price");

                    (yes_top_products && yes_name && no_price).into()
                })
                == Some(true)
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"version": 1, "control": "continue"}).to_string())
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    graphql:
                      request:
                        include:
                          body: false
                      analysis:
                        include:
                          body: [query, extensions]
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first:1) { name price } }", None, None)
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table"
              }
            ]
          }
        }
        "#);

    assert!(
        response.status().is_success(),
        "router should accept graphql.request body mutation and return successful response"
    );

    request_stage_mock.assert_async().await;
    analysis_stage_mock.assert_async().await;
}

#[ntex::test]
/// This test checks that graphql.request accepts missing query when mutating body,
/// so body patches are accepted, and the query stays intact.
async fn accepts_body_mutation_without_query() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage("graphql.request")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!(
            {
              "version": 1,
              "control": "continue",
              "body": {
                "extensions": {
                  "coprocessor": true
                }
              }
            })
            .to_string(),
        )
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    graphql:
                      request:
                        include:
                          body: false
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first:1) { name } }", None, None)
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table"
              }
            ]
          }
        }
        "#);

    assert!(
        response.status().is_success(),
        "router should accept graphql.request body mutation without query"
    );

    let products_requests = subgraphs.get_requests_log("products").unwrap_or_default();
    assert!(
        !products_requests.is_empty(),
        "subgraph request should not run when graphql.request body mutation is invalid"
    );

    request_stage_mock.assert_async().await;
}

#[ntex::test]
/// This test checks that graphql.request rejects empty query values in body mutations,
/// so malformed body patches fail before subgraph execution.
async fn rejects_body_mutation_with_empty_query() {
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage("graphql.request")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!(
              {
                "version": 1,
                "control": "continue",
                "body":{
                  "query": "   ",
                  "extensions": {
                    "coprocessor": true
                  }
                }
              }
            )
            .to_string(),
        )
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    graphql:
                      request:
                        include:
                          body: false
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first:1) { name } }", None, None)
        .await;

    // TODO: hide those errors from the client
    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
    {
      "errors": [
        {
          "extensions": {
            "code": "COPROCESSOR_INVALID_STAGE_BODY_ERROR"
          },
          "message": "Internal server error"
        }
      ]
    }
    "#);

    assert!(
        !response.status().is_success(),
        "router should reject graphql.request body mutation with empty query"
    );

    request_stage_mock.assert_async().await;
}

#[ntex::test]
/// This test checks that include.headers controls outbound payload only,
/// so graphql.request can still mutate headers even when outbound headers are not included.
async fn applies_headers_mutation_when_include_headers_false() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.request", |payload| {
            payload.get("headers").is_none()
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
              "version": 1,
              "control": "continue",
              "headers": {
                "x-coprocessor-stage": "request-mutated"
              }
            })
            .to_string(),
        )
        .expect(1)
        .create();

    let analysis_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.analysis", |payload| {
            let headers = payload.get("headers");
            headers
                .and_then(|headers| headers.get("x-coprocessor-stage"))
                .and_then(|value| sonic_rs::to_string(value).ok())
                .is_some_and(|value| {
                    value == "\"request-mutated\"" || value == "[\"request-mutated\"]"
                })
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"version": 1, "control": "continue"}).to_string())
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    graphql:
                      request:
                        include:
                          headers: false
                      analysis:
                        include:
                          headers: true
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first:1) { name } }", None, None)
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table"
              }
            ]
          }
        }
        "#);

    assert!(
        response.status().is_success(),
        "router should accept graphql.request headers mutation and return successful response"
    );

    request_stage_mock.assert_async().await;
    analysis_stage_mock.assert_async().await;
}

#[ntex::test]
async fn condition_check() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    graphql:
                      request:
                        condition:
                          expression: .request.method == "GET"
                        include:
                          body: [query, extensions]
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first:1) { name } }", None, None)
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table"
              }
            ]
          }
        }
        "#);
    assert!(
        response.status().is_success(),
        "router should return successful response"
    );
}
