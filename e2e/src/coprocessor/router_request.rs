use sonic_rs::json;

use crate::testkit::{coprocessor::TestCoprocessor, ClientResponseExt, TestRouter, TestSubgraphs};

#[ntex::test]
async fn short_circuit() {
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage("router.request")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
              "version": 1,
              "control": {
                "break": 401
              },
              "headers": {
                "x-custom-error": "unauthorized",
                "content-type": "application/json"
              },
              "body": "{\"error\": \"Unauthorized from coprocessor\"}"
            })
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
                    router:
                      request:
                        condition:
                          expression: .request.method == "POST"
                        include:
                          headers: true
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts { name } }", None, None)
        .await;

    assert_eq!(
        response.status().as_u16(),
        401,
        "router should return the status code from the coprocessor break control"
    );

    let header = response
        .headers()
        .get("x-custom-error")
        .and_then(|v| v.to_str().ok());
    assert_eq!(
        header,
        Some("unauthorized"),
        "router should apply headers provided during a break"
    );

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "error": "Unauthorized from coprocessor"
        }
        "#);

    request_stage_mock.assert_async().await;
}

#[ntex::test]
async fn mutates_headers() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage("router.request")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
              "version": 1,
              "control": "continue",
              "headers": {
                "x-coprocessor-router": "set-by-router-request",
                "content-type": "application/json"
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
                headers:
                  all:
                    request:
                      - propagate:
                          named: x-coprocessor-router
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    router:
                      request:
                        condition:
                          expression: .request.method == "POST"
                        include:
                          headers: true
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts { name } }", None, None)
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Table"
              },
              {
                "name": "Couch"
              },
              {
                "name": "Glass"
              },
              {
                "name": "Chair"
              },
              {
                "name": "TV"
              }
            ]
          }
        }
        "#);

    assert!(
        response.status().is_success(),
        "router should accept router request headers mutation and return successful response"
    );

    let products_requests = subgraphs.get_requests_log("products").unwrap_or_default();
    assert_eq!(
        products_requests.len(),
        1,
        "expected one request to products subgraph"
    );

    request_stage_mock.assert_async().await;
}
