#[cfg(test)]
mod cache_control_e2e_tests {

    use ntex::http;
    use reqwest::StatusCode;
    use sonic_rs::json;

    use crate::testkit::{some_header_map, ResponseLike, TestRouter, TestSubgraphs};

    // helper: read the cache-control response header as a string
    fn cache_control(res: &ntex::client::ClientResponse) -> Option<String> {
        res.header("cache-control").map(|v| {
            v.to_str()
                .expect("cache-control is not valid ascii")
                .to_string()
        })
    }

    // scenario 1: private data from one subgraph must poison the entire response
    // subgraph A (products): Cache-Control: max-age=0, no-cache, no-store, private
    // subgraph B (inventory): Cache-Control: public, max-age=300
    // expected: no-store, no-cache
    #[ntex::test]
    async fn private_data_must_not_be_cached() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("products") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            http::header::CACHE_CONTROL => "max-age=0, no-cache, no-store, private"
                        },
                    ))
                } else if req.path.contains("inventory") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            http::header::CACHE_CONTROL => "public, max-age=300"
                        },
                    ))
                } else {
                    None
                }
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

        // topProducts { upc inStock } hits both products and inventory
        let res = router
            .send_graphql_request(r#"{ topProducts(first: 1) { upc inStock } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");
        assert!(
            cc.contains("no-store") && cc.contains("no-cache"),
            "expected no-store, no-cache but got: {cc}"
        );
        assert!(
            !cc.contains("public"),
            "expected public to be absent but got: {cc}"
        );
    }

    // scenario 2: pick the shortest TTL across subgraphs
    // subgraph A (products): Cache-Control: public, max-age=500
    // subgraph B (inventory): Cache-Control: public, max-age=300
    // expected: public, max-age=300
    #[ntex::test]
    async fn pick_shortest_ttl() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("products") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            http::header::CACHE_CONTROL => "public, max-age=500"
                        },
                    ))
                } else if req.path.contains("inventory") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            http::header::CACHE_CONTROL => "public, max-age=300"
                        },
                    ))
                } else {
                    None
                }
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
            .send_graphql_request(r#"{ topProducts(first: 1) { upc inStock } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");
        assert!(cc.contains("public"), "expected public but got: {cc}");
        assert!(
            cc.contains("max-age=300"),
            "expected max-age=300 (shortest wins) but got: {cc}"
        );
        assert!(
            !cc.contains("max-age=500"),
            "expected max-age=500 to be absent but got: {cc}"
        );
    }

    // scenario 3: any subgraph graphql error disables caching entirely
    // subgraph A (products): returns a graphql-level error
    // subgraph B (inventory): Cache-Control: public, max-age=300
    // expected: no-store, no-cache, must-revalidate
    #[ntex::test]
    async fn subgraph_graphql_error_disables_caching() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("products") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        Some(
                            json!({
                                "errors": [{"message": "something went wrong in products"}]
                            })
                            .to_string(),
                        ),
                        some_header_map! {
                            http::header::CONTENT_TYPE => "application/json",
                            http::header::CACHE_CONTROL => "public, max-age=300"
                        },
                    ))
                } else if req.path.contains("inventory") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            http::header::CACHE_CONTROL => "public, max-age=300"
                        },
                    ))
                } else {
                    None
                }
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
            .send_graphql_request(r#"{ topProducts(first: 1) { upc inStock } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");
        assert!(
            cc.contains("no-store") && cc.contains("no-cache") && cc.contains("must-revalidate"),
            "expected no-store, no-cache, must-revalidate but got: {cc}"
        );
    }

    // scenario 4: mutations disable caching regardless of subgraph headers
    #[ntex::test]
    async fn mutations_disable_caching() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("products") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        Some(
                            json!({
                                "data": {"oneofTest": {"result": "ok"}}
                            })
                            .to_string(),
                        ),
                        some_header_map! {
                            http::header::CONTENT_TYPE => "application/json",
                            http::header::CACHE_CONTROL => "public, max-age=300"
                        },
                    ))
                } else {
                    None
                }
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
            .send_graphql_request(
                r#"mutation { oneofTest(input: {a: "x"}) { result } }"#,
                None,
                None,
            )
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");
        assert!(
            cc.contains("no-store") && cc.contains("no-cache") && cc.contains("must-revalidate"),
            "expected no-store, no-cache, must-revalidate for mutation but got: {cc}"
        );
    }

    // scenario 5: single subgraph, no cache-control header returned
    // expected: safe fallback no-store
    #[ntex::test]
    async fn no_subgraph_cache_control_emits_no_store() {
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

        let res = router
            .send_graphql_request(r#"{ me { name } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");
        assert!(
            cc.contains("no-store"),
            "expected no-store fallback but got: {cc}"
        );
    }

    // scenario 6: global fallback cache_control from config is used when no subgraph sends it
    #[ntex::test]
    async fn global_fallback_cache_control_from_config() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                headers:
                    all:
                        cache_control: "public, max-age=180"
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(r#"{ me { name } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");
        assert!(
            cc.contains("public") && cc.contains("max-age=180"),
            "expected public, max-age=180 from config fallback but got: {cc}"
        );
    }

    // scenario 7: must-revalidate from any subgraph is forwarded
    // subgraph returns: Cache-Control: public, max-age=60, must-revalidate
    // expected: max-age=60 and must-revalidate both present
    #[ntex::test]
    async fn must_revalidate_propagated() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("accounts") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            http::header::CACHE_CONTROL => "public, max-age=60, must-revalidate"
                        },
                    ))
                } else {
                    None
                }
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
            .send_graphql_request(r#"{ me { name } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");
        assert!(
            cc.contains("must-revalidate"),
            "expected must-revalidate to be forwarded but got: {cc}"
        );
        assert!(
            cc.contains("max-age=60"),
            "expected max-age=60 but got: {cc}"
        );
    }

    // scenario 8: public only when all subgraphs agree
    // subgraph A (products): Cache-Control: public, max-age=200
    // subgraph B (inventory): no Cache-Control header (omitted)
    // expected: not public (one subgraph did not assert public)
    #[ntex::test]
    async fn public_only_when_all_agree() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("products") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            http::header::CACHE_CONTROL => "public, max-age=200"
                        },
                    ))
                } else {
                    // inventory returns no cache-control
                    None
                }
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

        // hits both products and inventory
        let res = router
            .send_graphql_request(r#"{ topProducts(first: 1) { upc inStock } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");
        assert!(
            !cc.contains("public"),
            "expected public to be absent when not all subgraphs agree but got: {cc}"
        );
    }

    // scenario 9: subgraph-level pinned cache_control from config overrides what the subgraph sends
    #[ntex::test]
    async fn subgraph_pinned_cache_control_from_config() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("accounts") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            // subgraph would normally say private, but config pins it
                            http::header::CACHE_CONTROL => "private, no-store"
                        },
                    ))
                } else {
                    None
                }
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
                headers:
                    subgraphs:
                        accounts:
                            cache_control: "public, max-age=120"
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(r#"{ me { name } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");
        assert!(
            cc.contains("public") && cc.contains("max-age=120"),
            "expected pinned public, max-age=120 to win over subgraph private but got: {cc}"
        );
    }

    // scenario 10: http execution error (non-200 status) from subgraph forces no-store, no-cache, must-revalidate
    #[ntex::test]
    async fn subgraph_execution_error_disables_caching() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("products") {
                    Some(ResponseLike::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        None,
                        None,
                    ))
                } else {
                    None
                }
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
            .send_graphql_request(r#"{ topProducts(first: 1) { upc name } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");
        assert!(
            cc.contains("no-store") && cc.contains("no-cache") && cc.contains("must-revalidate"),
            "expected no-store, no-cache, must-revalidate on network error but got: {cc}"
        );
    }
}
