#[cfg(test)]
mod extensions_propagation_e2e_tests {
    use bytes::Bytes;
    use sonic_rs::{json, JsonValueTrait};

    use crate::testkit::{ClientResponseExt, ResponseLike, TestRouter, TestSubgraphs};

    fn json_response(body: sonic_rs::Value) -> ResponseLike {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        ResponseLike {
            status: axum::http::StatusCode::OK,
            headers,
            body: Some(Bytes::from(body.to_string())),
        }
    }

    #[ntex::test]
    async fn should_propagate_extensions_with_last_algorithm() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("accounts") {
                    return Some(json_response(json!({
                        "data": { "users": [{ "id": "1" }] },
                        "extensions": { "foo": "from-accounts" }
                    })));
                }
                None
            })
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                extensions:
                  propagate:
                    algorithm: last
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "expected 200 OK");

        let body = res.json_body().await;
        assert_eq!(
            body["extensions"]["foo"],
            json!("from-accounts"),
            "expected subgraph extension to be propagated to client"
        );
    }

    #[ntex::test]
    async fn should_merge_extensions_with_first_algorithm() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                // accounts is fetched first (sequence node), products second
                if req.path.contains("accounts") {
                    return Some(json_response(json!({
                        "data": { "users": [{ "id": "1" }] },
                        "extensions": { "winner": "accounts" }
                    })));
                }
                if req.path.contains("products") {
                    return Some(json_response(json!({
                        "data": { "topProducts": [{ "name": "Widget" }] },
                        "extensions": { "winner": "products" }
                    })));
                }
                None
            })
            .build()
            .start()
            .await;

        // issue a request where the plan executes accounts before products
        // (sequence node) so the order is deterministic
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                extensions:
                  propagate:
                    algorithm: first
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "expected 200 OK");
        let body = res.json_body().await;
        assert_eq!(
            body["extensions"]["winner"],
            json!("accounts"),
            "first algorithm should keep the first value seen"
        );
    }

    #[ntex::test]
    async fn should_merge_extensions_with_append_algorithm() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("accounts") {
                    return Some(json_response(json!({
                        "data": { "users": [{ "id": "1" }] },
                        "extensions": { "trace": "accounts" }
                    })));
                }
                None
            })
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                extensions:
                  propagate:
                    algorithm: append
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "expected 200 OK");
        let body = res.json_body().await;
        // append always produces an array, even with a single value
        assert_eq!(
            body["extensions"]["trace"],
            json!(["accounts"]),
            "append algorithm should always produce an array"
        );
    }

    #[ntex::test]
    async fn should_not_propagate_extensions_when_config_is_absent() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("accounts") {
                    return Some(json_response(json!({
                        "data": { "users": [{ "id": "1" }] },
                        "extensions": { "secret": "should-not-appear" }
                    })));
                }
                None
            })
            .build()
            .start()
            .await;

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

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "expected 200 OK");
        let body = res.json_body().await;
        assert!(
            body["extensions"]["secret"].is_null(),
            "subgraph extensions must not leak without propagation config"
        );
    }

    #[ntex::test]
    async fn should_respect_allow_list() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("accounts") {
                    return Some(json_response(json!({
                        "data": { "users": [{ "id": "1" }] },
                        "extensions": {
                            "allowed": "yes",
                            "blocked": "no"
                        }
                    })));
                }
                None
            })
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                extensions:
                  propagate:
                    algorithm: last
                    allow: [allowed]
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "expected 200 OK");
        let body = res.json_body().await;
        assert_eq!(
            body["extensions"]["allowed"],
            json!("yes"),
            "allowed key should be propagated"
        );
        assert!(
            body["extensions"]["blocked"].is_null(),
            "key not in allow list should be dropped"
        );
    }

    #[ntex::test]
    async fn should_never_propagate_reserved_query_plan_key() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("accounts") {
                    return Some(json_response(json!({
                        "data": { "users": [{ "id": "1" }] },
                        "extensions": { "queryPlan": "subgraph-injected" }
                    })));
                }
                None
            })
            .build()
            .start()
            .await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                extensions:
                  propagate:
                    algorithm: last
                    allow: [queryPlan] # even if explicitly allowed
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert!(res.status().is_success(), "expected 200 OK");
        let body = res.json_body().await;
        // queryPlan is reserved; a subgraph cannot inject it even with all-keys propagation
        assert_ne!(
            body["extensions"]["queryPlan"],
            json!("subgraph-injected"),
            "reserved queryPlan key must never be overwritten by a subgraph"
        );
    }
}
