use sonic_rs::JsonValueTrait;
use sonic_rs::{json, pointer};

use crate::testkit::{coprocessor::TestCoprocessor, ClientResponseExt, TestRouter, TestSubgraphs};

mod basic {
    use super::*;

    #[ntex::test]
    async fn context_false_omits_context_field() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage_with_matcher("graphql.request", |payload| {
                payload.get("context").is_none()
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
                            context: false
                  "#
            ))
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request(
                "query MyContextFalse { topProducts(first: 1) { name } }",
                None,
                None,
            )
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
        request_stage_mock.assert_async().await;
    }

    #[ntex::test]
    async fn context_list_sends_only_selected_reserved_keys() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage_with_matcher("graphql.request", |payload| {
                let context = match payload.get("context") {
                    Some(context) => context,
                    None => return false,
                };

                context
                    .pointer(&["hive::operation::name"])
                    .and_then(|value| value.as_str())
                    == Some("Example")
                    && context.pointer(&["hive::operation::kind"]).is_none()
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
                            context: ["hive::operation::name"]
                  "#
            ))
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request(
                "query Example { topProducts(first: 1) { name } }",
                None,
                None,
            )
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
        request_stage_mock.assert_async().await;
    }

    #[ntex::test]
    async fn context_patch_updates_key_not_sent_in_request_stage() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage_with_matcher("graphql.request", |payload| {
                payload
                    .pointer(&["context", "hive::operation::name"])
                    .and_then(|value| value.as_str())
                    == Some("Example")
                    && payload.pointer(&["context", "custom::b"]).is_none()
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "version": 1,
                    "control": "continue",
                    "context": {
                        "custom::b": "patched-value"
                    }
                })
                .to_string(),
            )
            .expect(1)
            .create();

        let router_response_stage_mock = coprocessor
            .mock_stage_with_matcher("router.response", |payload| {
                println!("router.response: {:?}", payload);
                payload
                    .pointer(&["context", "custom::b"])
                    .and_then(|value| value.as_str())
                    == Some("patched-value")
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
                            context: ["hive::operation::name"]
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
                "query Example { topProducts(first: 1) { name } }",
                None,
                None,
            )
            .await;

        request_stage_mock.assert_async().await;
        router_response_stage_mock.assert_async().await;

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
    }

    #[ntex::test]
    async fn context_reserved_key_mutation_is_rejected() {
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage("graphql.request")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "version": 1,
                    "control": "continue",
                    "context": {
                        "hive::operation::name": "Overridden"
                    }
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
                      graphql:
                        request:
                          include:
                            context: true
                  "#
            ))
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request(
                "query MyImmutableContext { topProducts { name } }",
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "errors": [
            {
              "extensions": {
                "code": "REQUEST_CONTEXT_ERROR"
              },
              "message": "Internal server error"
            }
          ]
        }
        "#);

        assert!(!response.status().is_success());
        request_stage_mock.assert_async().await;
    }
}

mod progressive_override {
    use super::*;

    #[ntex::test]
    async fn multi_stage_update() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage("graphql.request")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                // graphql.request sets `-0` flag
                json!({
                    "version": 1,
                    "control": "continue",
                    "context": {
                        "hive::progressive_override::labels_to_override": ["my-flag-0"]
                    }
                })
                .to_string(),
            )
            .expect(1)
            .create();

        let graphql_analysis_stage_mock = coprocessor
            .mock_stage_with_matcher("graphql.analysis", |payload| {
                payload
                    .pointer(&pointer![
                        "context",
                        "hive::progressive_override::labels_to_override",
                        0,
                    ])
                    // graphql.analysis expects `-0` flag
                    .is_some_and(|value| value.as_str() == Some("my-flag-0"))
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                // graphql.analysis sets `-1` flag
                json!({
                  "version": 1,
                  "control": "continue",
                  "context": {
                      "hive::progressive_override::labels_to_override": ["my-flag-1"]
                  }
                })
                .to_string(),
            )
            .expect(1)
            .create();

        let router_response_stage_mock = coprocessor
            .mock_stage_with_matcher("router.response", |payload| {
                payload
                    .pointer(&pointer![
                        "context",
                        "hive::progressive_override::labels_to_override",
                        0,
                    ])
                    // router.response expects `-1` flag
                    .is_some_and(|value| value.as_str() == Some("my-flag-1"))
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
                        context: true
                    analysis:
                      include:
                        context: true
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
            .send_graphql_request("{ topProducts(first: 1) { name } }", None, None)
            .await;

        request_stage_mock.assert_async().await;
        graphql_analysis_stage_mock.assert_async().await;
        router_response_stage_mock.assert_async().await;

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
    }

    #[ntex::test]
    async fn labels_propagated_to_next_stages() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage("graphql.request")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "version": 1,
                    "control": "continue",
                    "context": {
                        "hive::progressive_override::labels_to_override": ["my-flag"]
                    }
                })
                .to_string(),
            )
            .expect(1)
            .create();

        let router_response_stage_mock = coprocessor
            .mock_stage_with_matcher("router.response", |payload| {
                payload
                    .pointer(&pointer![
                        "context",
                        "hive::progressive_override::labels_to_override",
                        0,
                    ])
                    .is_some_and(|value| value.as_str() == Some("my-flag"))
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
                        context: true
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
            .send_graphql_request("{ topProducts(first: 1) { name } }", None, None)
            .await;

        request_stage_mock.assert_async().await;
        router_response_stage_mock.assert_async().await;

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
    }

    #[ntex::test]
    async fn unresolved_labels_mutation_rejected() {
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage("graphql.request")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "version": 1,
                    "control": "continue",
                    "context": {
                        "hive::progressive_override::unresolved_labels": ["my-flag"]
                    }
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
                  graphql:
                    request:
                      include:
                        context: true
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
          "errors": [
            {
              "extensions": {
                "code": "REQUEST_CONTEXT_ERROR"
              },
              "message": "Internal server error"
            }
          ]
        }
        "#);

        assert!(!response.status().is_success());
        request_stage_mock.assert_async().await;
    }

    #[ntex::test]
    async fn type_mismatch_rejected() {
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let request_stage_mock = coprocessor
            .mock_stage("graphql.request")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "version": 1,
                    "control": "continue",
                    "context": {
                        "hive::progressive_override::labels_to_override": "my-flag"
                    }
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
                  graphql:
                    request:
                      include:
                        context: true
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
          "errors": [
            {
              "extensions": {
                "code": "REQUEST_CONTEXT_ERROR"
              },
              "message": "Internal server error"
            }
          ]
        }
        "#);

        assert!(!response.status().is_success());
        request_stage_mock.assert_async().await;
    }
}
