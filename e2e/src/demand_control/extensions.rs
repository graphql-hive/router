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
                      "result": "COST_OK",
                      "estimatedCostBySubgraph": {
                        "accounts": 1
                      },
                      "resultBySubgraph": {
                        "accounts": "COST_OK"
                      },
                      "formulaCacheHit": false,
                      "estimatedFormulaBySubgraph": {
                        "accounts": "1"
                      },
                      "maxCost": 100,
                      "actual": 1,
                      "delta": 0,
                      "actualCostBySubgraph": {
                        "accounts": 1
                      }
                    }
                  }
                }
                "#);
    }
    #[ntex::test]
    async fn exposes_formula_cache_hit_in_cost_extension() {
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

        let query = r#"
            query {
              book(id: 1) {
                title
              }
            }
        "#;

        let first = router.send_graphql_request(query, None, None).await;
        let first_json = first.json_body().await;
        assert_eq!(
            first_json["extensions"]["cost"]["formulaCacheHit"].as_bool(),
            Some(false),
            "first request should miss formula cache"
        );

        let second = router.send_graphql_request(query, None, None).await;
        let second_json = second.json_body().await;
        assert_eq!(
            second_json["extensions"]["cost"]["formulaCacheHit"].as_bool(),
            Some(true),
            "second request should hit formula cache"
        );
    }
    #[ntex::test]
    async fn exposes_summed_estimated_formula_for_subgraph_extension_metadata() {
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
                query SummedFormulaMetadata($includeAuthorName: Boolean!) {
                  me {
                    id
                    reviews {
                      author {
                        name @include(if: $includeAuthorName)
                      }
                    }
                  }
                }
                "#,
                Some(json!({ "includeAuthorName": true })),
                None,
            )
            .await;

        let json = res.json_body().await;
        let accounts_formula = json["extensions"]["cost"]["estimatedFormulaBySubgraph"]["accounts"]
            .as_str()
            .expect("accounts formula should be present");

        // accounts subgraph runs two fetches:
        //   1) `{ me { __typename id } }` → User(1) + __typename(0) + id(0) = 1
        //   2) entity flatten `_entities { ... on User { ... } }` → 1 entity × User(1) = 1
        // __typename is treated as zero-cost during estimated cost compilation.
        assert_eq!(accounts_formula, "2");
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
                      "result": "COST_OK",
                      "estimatedCostBySubgraph": {
                        "books": 8
                      },
                      "resultBySubgraph": {
                        "books": "COST_OK"
                      },
                      "formulaCacheHit": false,
                      "estimatedFormulaBySubgraph": {
                        "books": "(inputCost($input) + ($input.pagination.first * 2))"
                      },
                      "maxCost": 1000,
                      "actual": 6,
                      "delta": -2,
                      "actualCostBySubgraph": {
                        "books": 6
                      }
                    }
                  }
                }
                "#);
    }
}
