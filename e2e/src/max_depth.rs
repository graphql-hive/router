#[cfg(test)]
mod max_depth_e2e_tests {
    // builds `query { ...F1 } fragment F1 on Query { ...F2 } ... fragment FN on Query { __typename }`
    // brace depth never exceeds 2, so the tokenizer's recursion_limit=50 is irrelevant.
    // each spread without flatten_fragments adds 1 to the validated depth, so N fragments -> depth N.
    fn acyclic_fragment_chain(n: usize) -> String {
        let mut s = String::from("query { ...F1 }\n");
        for i in 1..n {
            s.push_str(&format!("fragment F{i} on Query {{ ...F{} }}\n", i + 1));
        }
        s.push_str(&format!("fragment F{n} on Query {{ __typename }}\n"));
        s
    }

    use crate::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};

    const QUERY: &'static str = r#"
            query {
                me {
                    name
                    reviews {
                        body
                    }
                }
            }
        "#;

    #[ntex::test]
    async fn allows_query_within_max_depth() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        limits:
            max_depth:
                n: 3
        "#,
            )
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "data": {
            "me": {
              "name": "Uri Goldshtein",
              "reviews": [
                {
                  "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum."
                },
                {
                  "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi"
                }
              ]
            }
          }
        }
        "###);
    }

    #[ntex::test]
    async fn rejects_query_exceeding_max_depth() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        limits:
            max_depth:
                n: 1
        "#,
            )
            .build()
            .start()
            .await;

        let res = router.send_graphql_request(QUERY, None, None).await;
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "errors": [
            {
              "message": "Query depth limit exceeded.",
              "extensions": {
                "code": "MAX_DEPTH_EXCEEDED"
              }
            }
          ]
        }
        "###);
    }

    #[ntex::test]
    async fn unknown_fragments() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
        supergraph:
            source: file
            path: supergraph.graphql
        limits:
            max_depth:
                n: 3
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
                    ...UnknownFragment
                }
            }
            "#,
                None,
                None,
            )
            .await;
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "errors": [
            {
              "message": "Unknown fragment \"UnknownFragment\".",
              "locations": [
                {
                  "line": 4,
                  "column": 24
                }
              ],
              "extensions": {
                "code": "KnownFragmentNames"
              }
            }
          ]
        }
        "###);
    }

    #[ntex::test]
    async fn rejects_acyclic_fragment_chain_exceeding_max_depth() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                limits:
                    max_depth:
                        n: 10
                "#,
            )
            .build()
            .start()
            .await;

        // N=200 fragments, each `on Query { ...FN+1 }` - brace depth <=2, but validated depth=200
        let query = acyclic_fragment_chain(200);
        let res = router.send_graphql_request(&query, None, None).await;
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "errors": [
            {
              "message": "Query depth limit exceeded.",
              "extensions": {
                "code": "MAX_DEPTH_EXCEEDED"
              }
            }
          ]
        }
        "###);
    }

    #[ntex::test]
    async fn acyclic_fragment_chain_without_max_depth_does_not_crash_router() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                "#,
            )
            .build()
            .start()
            .await;

        // N=50_000 acyclic fragment chain, brace depth <=2 (bypasses tokenizer limit),
        // but normalization recurses N-deep and overflows the stack
        let query = acyclic_fragment_chain(50_000);
        let res = router.send_graphql_request(&query, None, None).await;
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "errors": [
            {
              "message": "Query depth limit exceeded.",
              "extensions": {
                "code": "MAX_DEPTH_EXCEEDED"
              }
            }
          ]
        }
        "###);
    }

    #[ntex::test]
    async fn allows_short_acyclic_fragment_chain_within_max_depth() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                limits:
                    max_depth:
                        n: 10
                "#,
            )
            .build()
            .start()
            .await;

        // N=3 fragments, depth=3, well within n=10
        let query = acyclic_fragment_chain(3);
        let res = router.send_graphql_request(&query, None, None).await;
        insta::assert_snapshot!(res.json_body_string_pretty().await, @r###"
        {
          "data": {
            "__typename": "Query"
          }
        }
        "###);
    }
}
