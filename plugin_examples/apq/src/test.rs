#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, TestRouterBuilder, TestSubgraphsBuilder};
    use hive_router::{http::StatusCode, ntex, sonic_rs};

    #[ntex::test]
    async fn sends_not_found_error_if_query_missing() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/apq/router.config.yaml")
            .register_plugin::<crate::plugin::APQPlugin>()
            .build()
            .start()
            .await;

        let res = router
            .serv()
            .post(router.graphql_path())
            .header("content-type", "application/json")
            .send_json(&sonic_rs::json!({
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": "ecf4edb46db40b5132295c0291d62fb65d6759a9eedfa4d5d612dd5ec54a6b38",
                    },
                },
            }))
            .await
            .unwrap();

        assert_eq!(
            res.json_body().await,
            sonic_rs::json!({
                "errors": [
                    {
                        "message": "PersistedQueryNotFound",
                        "extensions": {
                            "code": "PERSISTED_QUERY_NOT_FOUND"
                        }
                    }
                ]
            }),
            "Expected PersistedQueryNotFound error"
        );
    }

    #[ntex::test]
    async fn saves_persisted_query() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        let router = TestRouterBuilder::new()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/apq/router.config.yaml")
            .register_plugin::<crate::plugin::APQPlugin>()
            .build()
            .start()
            .await;

        let query = "{ users { id } }";
        let sha256_hash = "ecf4edb46db40b5132295c0291d62fb65d6759a9eedfa4d5d612dd5ec54a6b38";

        let res = router
            .serv()
            .post(router.graphql_path())
            .header("content-type", "application/json")
            .send_json(&sonic_rs::json!({
                "query": query,
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": sha256_hash,
                    },
                },
            }))
            .await
            .unwrap();

        assert_eq!(
            res.status(),
            StatusCode::OK,
            "Expected 200 OK when sending full query"
        );

        // Now send only the hash and expect it to be found
        let res = router
            .serv()
            .post(router.graphql_path())
            .header("content-type", "application/json")
            .send_json(&sonic_rs::json!({
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": sha256_hash,
                    },
                },
            }))
            .await
            .unwrap();

        assert!(
            res.status().is_success(),
            "Expected 200 OK when sending persisted query hash"
        );

        let subgraph_requests = subgraphs
            .get_requests_log("accounts")
            .expect("expected requests sent to accounts subgraph");
        assert_eq!(
            subgraph_requests.len(),
            2,
            "expected 2 requests to accounts subgraph"
        );
    }
}
