#[cfg(test)]
mod extensions_tests {
    use super::super::common::*;

    #[ntex::test]
    async fn includes_cost_metadata_in_response_extensions_when_enabled() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        demand_control:
            enabled: true
            mode: enforce
            strategy:
              static_estimated:
                max: 100
            include_extension_metadata: true
        "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query {
                    me {
                        name
                    }
                }
                "#,
                None,
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
                {
                  "data": {
                    "me": {
                      "name": "Uri Goldshtein"
                    }
                  },
                  "extensions": {
                    "cost": {
                      "estimated": 1,
                      "max": 100,
                      "actual": 1
                    }
                  }
                }
                "#);
    }

    #[ntex::test]
    async fn exposes_variable_placeholders_in_estimated_formula_metadata() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph_demand_control.graphql

                demand_control:
                    enabled: true
                    mode: enforce
                    strategy:
                      static_estimated:
                        max: 1000
                    include_extension_metadata: true
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                query SearchFormulaHasVariable($input: SearchInput!) {
                  search(input: $input) {
                    title
                    author {
                      name
                    }
                  }
                }
                "#,
                Some(json!({
                    "input": {
                        "pagination": { "first": 3 }
                    }
                })),
                None,
            )
            .await;

        insta::assert_snapshot!(res.json_body_string_pretty().await, @r#"
                {
                  "data": {
                    "search": [
                      {
                        "title": "The Mystery at Midnight",
                        "author": {
                          "name": "Alice Smith"
                        }
                      },
                      {
                        "title": "Science Fiction Dreams",
                        "author": {
                          "name": "Bob Johnson"
                        }
                      },
                      {
                        "title": "Classic Fiction",
                        "author": {
                          "name": "Charlie Brown"
                        }
                      }
                    ]
                  },
                  "extensions": {
                    "cost": {
                      "estimated": 8,
                      "max": 1000,
                      "actual": 6
                    }
                  }
                }
                "#);
    }
}
