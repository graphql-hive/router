use sonic_rs::JsonValueTrait;
use sonic_rs::{json, pointer};

use crate::testkit::{coprocessor::TestCoprocessor, ClientResponseExt, TestRouter, TestSubgraphs};

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
                json!({
                    "version": 1,
                    "control": "continue",
                    "context": {
                        "hive::progressive_override::labels_to_override": ["launchDarkly:my-flag-0"]
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
                    .is_some_and(|value| value.as_str() == Some("launchDarkly:my-flag-0"))
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                  "version": 1,
                  "control": "continue",
                  "context": {
                      "hive::progressive_override::labels_to_override": ["launchDarkly:my-flag-1"]
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
                    .is_some_and(|value| value.as_str() == Some("launchDarkly:my-flag-1"))
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
            .send_graphql_request(
                "query MyProgressiveOverrideContextPatch { topProducts(first: 1) { name } }",
                None,
                None,
            )
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
                        "hive::progressive_override::labels_to_override": ["launchDarkly:my-flag"]
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
                    .is_some_and(|value| value.as_str() == Some("launchDarkly:my-flag"))
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
            .send_graphql_request(
                "query MyProgressiveOverrideContextPatch { topProducts(first: 1) { name } }",
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
                        "hive::progressive_override::unresolved_labels": ["launchDarkly:my-flag"]
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
                "query MyImmutableProgressiveOverrideContext { topProducts { name } }",
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
            "message": "request context error: request-context key 'hive::progressive_override::unresolved_labels' cannot be mutated externally"
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
                        "hive::progressive_override::labels_to_override": "launchDarkly:my-flag"
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
                "query MyProgressiveOverrideTypeMismatch { topProducts { name } }",
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
            "message": "request context error: reserved request-context key 'hive::progressive_override::labels_to_override' has an invalid type: expected array of strings or null"
          }
        ]
      }
      "#);

        assert!(!response.status().is_success());
        request_stage_mock.assert_async().await;
    }
}
