use sonic_rs::json;
use sonic_rs::JsonValueTrait;

use crate::testkit::{coprocessor::TestCoprocessor, ClientResponseExt, TestRouter, TestSubgraphs};

#[ntex::test]
async fn mutates_headers_and_body() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let response_stage_mock = coprocessor
        .mock_stage("router.response")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
              "version": 1,
              "control": "continue",
              "headers": {
                "x-coprocessor-response": "set-by-router-response",
                "content-type": "application/json"
              },
              "body": "{\"data\": {\"topProducts\": [{\"name\": \"Intercepted\"}]}}"
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
                    router:
                      response:
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

    assert!(
        response.status().is_success(),
        "response status should be success"
    );

    let header = response
        .headers()
        .get("x-coprocessor-response")
        .and_then(|v| v.to_str().ok());
    assert_eq!(
        header,
        Some("set-by-router-response"),
        "router should apply headers provided by router.response stage"
    );

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": {
            "topProducts": [
              {
                "name": "Intercepted"
              }
            ]
          }
        }
        "#);

    response_stage_mock.assert_async().await;
}

#[ntex::test]
async fn includes_graphql_operation_context() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let response_stage_mock = coprocessor
        .mock_stage_with_matcher("router.response", |payload| {
            payload
                .pointer(&["context", "hive::operation::name"])
                .and_then(|value| value.as_str())
                == Some("MyRouterResponseContext")
                && payload
                    .pointer(&["context", "hive::operation::kind"])
                    .and_then(|value| value.as_str())
                    == Some("query")
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
                    router:
                      response:
                        include:
                          context: true
                "#
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request(
            "query MyRouterResponseContext { topProducts { name } }",
            None,
            None,
        )
        .await;

    let _ = response;
    response_stage_mock.assert_async().await;
}
