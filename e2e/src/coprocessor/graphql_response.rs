use sonic_rs::json;

use crate::testkit::coprocessor::TestCoprocessor;
use crate::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};

#[ntex::test]
/// This test checks that graphql.response accepts json body
async fn graphql_response_accepts_json_body() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage("graphql.response")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
              "version": 1,
              "control": "continue",
              "body": {
                "data": null,
                "errors": [{
                  "message": "hello from coprocessor"
                }]
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
                      response:
                        include:
                          headers: true
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
          "query": "{ topProducts { name } }"
        }))
        .await
        .expect("failed to send graphql request");

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": null,
          "errors": [
            {
              "message": "hello from coprocessor"
            }
          ]
        }
        "#);
    assert!(
        response.status().is_success(),
        "router should return successful response"
    );

    request_stage_mock.assert_async().await;
}

#[ntex::test]
/// This test checks that graphql.response accepts string body
async fn graphql_response_accepts_string_body() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let request_stage_mock = coprocessor
        .mock_stage("graphql.response")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
              "version": 1,
              "control": "continue",
              "body": json!({
                "data": null,
                "errors": [{
                  "message": "hello from coprocessor"
                }]
              }).to_string()
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
                      response:
                        include:
                          headers: true
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
          "query": "{ topProducts { name } }"
        }))
        .await
        .expect("failed to send graphql request");

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "data": null,
          "errors": [
            {
              "message": "hello from coprocessor"
            }
          ]
        }
        "#);
    assert!(
        response.status().is_success(),
        "router should return successful response"
    );

    request_stage_mock.assert_async().await;
}
