#[cfg(test)]
mod query_plan_cache_e2e_tests {
    use sonic_rs::json;

    use crate::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};

    // `"u1"` (a String argument) and `$u1` (a Variable) once hashed identically, so these
    // two operations collided on one query plan cache key while needing different plans: the
    // first resolves `a` from the variable and `b` from the literal, the second the other way
    // around. The injective AST hash now gives them distinct keys; this test guards the
    // end-to-end behavior so a regression in the hash (or the cache) can't resurface it.
    const VARIABLE_FIRST: &str =
        r#"query($u1: ID!) { a: user(id: $u1) { id } b: user(id: "u1") { id } }"#;
    const LITERAL_FIRST: &str =
        r#"query($u1: ID!) { a: user(id: "u1") { id } b: user(id: $u1) { id } }"#;

    /// One operation must never be served the plan built for the other. Here `$u1 = "1"` is a
    /// real user and `"u1"` is not, so the two operations must return mirror-image results.
    #[ntex::test]
    async fn does_not_reuse_a_cached_plan_built_for_a_different_operation() {
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

        let variables = Some(json!({ "u1": "1" }));

        // Plans and caches the first operation under the shared key.
        let first = router
            .send_graphql_request(VARIABLE_FIRST, variables.clone(), None)
            .await
            .json_body()
            .await;
        assert_eq!(
            first,
            json!({ "data": { "a": { "id": "1" }, "b": null } }),
            "unexpected response for the first operation"
        );

        // Same cache key, different operation: it must not be served the first plan.
        let second = router
            .send_graphql_request(LITERAL_FIRST, variables, None)
            .await
            .json_body()
            .await;
        assert_eq!(
            second,
            json!({ "data": { "a": null, "b": { "id": "1" } } }),
            "the second operation must not execute the plan cached for the first"
        );
    }

    /// `user(id: "1")` and `user(id: "2")` share one plan, so each
    /// request has to bring its own value along to the subgraph.
    #[ntex::test]
    async fn extracted_literals_keep_their_own_values() {
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

        for id in ["1", "2"] {
            let query = format!(r#"{{ user(id: "{id}") {{ id }} }}"#);
            let response = router
                .send_graphql_request(&query, None, None)
                .await
                .json_body()
                .await;
            assert_eq!(response, json!({ "data": { "user": { "id": id } } }));
        }

        let bodies: Vec<String> = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph")
            .iter()
            .map(|request| String::from_utf8_lossy(request.body.as_ref().unwrap()).to_string())
            .collect();
        insta::assert_snapshot!(bodies.join("\n"), @r#"
        {"query":"query($_v0:ID!){user(id: $_v0){id}}","variables":{"_v0":"1"}}
        {"query":"query($_v0:ID!){user(id: $_v0){id}}","variables":{"_v0":"2"}}
        "#);
    }
}
