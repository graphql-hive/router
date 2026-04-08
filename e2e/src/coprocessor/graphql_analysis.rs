#[cfg(test)]
mod coprocessor_graphql_analysis_e2e_tests {
    use sonic_rs::{json, JsonValueTrait};

    use crate::testkit::{
        coprocessor::TestCoprocessor, ClientResponseExt, TestRouter, TestSubgraphs,
    };

    #[ntex::test]
    /// This test checks that graphql.analysis treats body as read-only,
    /// so returning a body mutation from coprocessor causes the request to fail before subgraph execution.
    async fn rejects_body_mutation_from_coprocessor() {
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let analysis_stage_mock = coprocessor
            .mock_stage("graphql.analysis")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                  "version": 1,
                  "control": "continue",
                  "body": {
                    "query": "{ topProducts { name } }"
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
                      analysis:
                        include:
                          body: [query]
                "#
            ))
            .build()
            .start()
            .await;

        let response = router
            .send_graphql_request("{ topProducts { name } }", None, None)
            .await;

        // TODO: hide those errors from the client
        insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
        {
          "errors": [
            {
              "extensions": {
                "code": "COPROCESSOR_FAILURE"
              },
              "message": "coprocessor graphql.analysis stage cannot mutate 'body'"
            }
          ]
        }
        "#);

        assert!(
            !response.status().is_success(),
            "router should reject analysis body mutation and return an error response"
        );

        analysis_stage_mock.assert_async().await;
    }

    #[ntex::test]
    /// This test checks that include.headers controls outbound payload only,
    /// so graphql.analysis can still mutate headers even when outbound headers are not included.
    async fn applies_headers_mutation_when_include_headers_false() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let analysis_stage_mock = coprocessor
            .mock_stage_with_matcher("graphql.analysis", |payload| {
                payload.get("headers").is_none()
            })
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                  "version": 1,
                  "control": "continue",
                  "headers": {
                    "x-coprocessor-analysis": "set-by-analysis"
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
                          named: x-coprocessor-analysis
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    graphql:
                      analysis:
                        include:
                          headers: false
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
            "router should accept analysis headers mutation and return successful response"
        );

        let products_requests = subgraphs.get_requests_log("products").unwrap_or_default();
        assert_eq!(
            products_requests.len(),
            1,
            "expected one request to products subgraph"
        );

        let propagated_header = products_requests[0]
            .headers
            .get("x-coprocessor-analysis")
            .and_then(|value| value.to_str().ok());
        assert_eq!(
            propagated_header,
            Some("set-by-analysis"),
            "analysis-mutated header should be propagated to subgraph request"
        );

        analysis_stage_mock.assert_async().await;
    }

    #[ntex::test]
    async fn short_circuit_preserves_headers() {
        let mut coprocessor = TestCoprocessor::new().await;
        let host = coprocessor.host_with_port();

        let analysis_stage_mock = coprocessor
            .mock_stage("graphql.analysis")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                  "version": 1,
                  "control": { "break": 401 },
                  "headers": {
                    "content-type": "application/json",
                    "x-analysis-error": "unauthorized"
                  },
                  "body": {
                    "errors": [
                      {
                        "message": "unauthorized from analysis"
                      }
                    ]
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
                      analysis:
                        include:
                          body: [query]
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
            .get("x-analysis-error")
            .and_then(|v| v.to_str().ok());
        assert_eq!(
            header,
            Some("unauthorized"),
            "router should apply headers provided during a break in analysis stage"
        );

        analysis_stage_mock.assert_async().await;
    }
}
