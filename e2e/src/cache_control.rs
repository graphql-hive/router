#[cfg(test)]
mod cache_control_e2e_tests {

    use ntex::http;
    use reqwest::StatusCode;
    use sonic_rs::json;

    use crate::testkit::{some_header_map, ResponseLike, Started, TestRouter, TestSubgraphs};
    use hive_router::{
        async_trait,
        http::{HeaderName, HeaderValue},
        plugins::{
            hooks::{
                on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
                on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
            },
            plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
        },
        GraphQLError,
    };

    // inserts a cacheable cache-control from the on_execute end hook, and adds a graphql
    // error there too when the client sends x-add-error. used to check that the router's
    // no-store rules for mutations and errors still hold over plugin-written values
    #[derive(Default)]
    struct CacheablePlugin;

    #[async_trait]
    impl RouterPlugin for CacheablePlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_cacheable"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_execute<'exec>(
            &'exec self,
            payload: OnExecuteStartHookPayload<'exec>,
        ) -> OnExecuteStartHookResult<'exec> {
            let add_error = payload
                .router_http_request
                .headers
                .contains_key("x-add-error");
            payload.on_end(move |mut payload| {
                if add_error {
                    payload.with_error(GraphQLError::from_message_and_code(
                        "plugin error",
                        "PLUGIN_ERROR",
                    ));
                }
                payload.insert_response_header(
                    HeaderName::from_static("cache-control"),
                    HeaderValue::from_static("public, max-age=60"),
                );
                payload.proceed()
            })
        }
    }

    async fn start_router_with_cacheable_plugin(
        subgraphs: &TestSubgraphs<Started>,
    ) -> TestRouter<Started> {
        TestRouter::builder()
            .with_subgraphs(subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                plugins:
                    test_cacheable:
                        enabled: true
                "#,
            )
            .register_plugin::<CacheablePlugin>()
            .build()
            .start()
            .await
    }

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
                headers:
                    all:
                        response:
                            - propagate:
                                named: cache-control
                                algorithm: append
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
                headers:
                    all:
                        response:
                            - propagate:
                                named: cache-control
                                algorithm: append
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
                headers:
                    all:
                        response:
                            - propagate:
                                named: cache-control
                                algorithm: append
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
                headers:
                    all:
                        response:
                            - propagate:
                                named: cache-control
                                algorithm: append
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"mutation { oneofTest(input: { string: "dummy" }) { id } }"#,
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

    // scenario 5: no subgraph sends cache-control and no rule configured
    // expected: no cache-control header emitted (no implicit fallback)
    #[ntex::test]
    async fn no_subgraph_cache_control_emits_no_header() {
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

        // no propagate rule and no insert rule: cache-control is absent
        // no implicit no-store fallback; add an insert rule if a safe default is needed
        assert!(
            cache_control(&res).is_none(),
            "expected no cache-control header when nothing is configured but got: {:?}",
            cache_control(&res)
        );
    }

    // scenario 6: global insert cache-control rule
    #[ntex::test]
    async fn global_insert_cache_control_from_config() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("accounts") {
                    // router shouldnt care, we're overriding
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            http::header::CACHE_CONTROL => "no-cache"
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
                    all:
                        response:
                            - insert:
                                name: cache-control
                                value: "public, max-age=180"
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
                headers:
                    all:
                        response:
                            - propagate:
                                named: cache-control
                                algorithm: append
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
                headers:
                    all:
                        response:
                            - propagate:
                                named: cache-control
                                algorithm: append
                "#,
            )
            .build()
            .start()
            .await;

        // hits both products and inventory; only products sends cache-control
        let res = router
            .send_graphql_request(r#"{ topProducts(first: 1) { upc inStock } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).unwrap_or_default();
        assert!(!cc.contains("public"), "didnt expect public cache-control");
        assert!(
            cc.contains("max-age=200"),
            "expected max-age=200 from products but got: {cc}"
        )
    }

    // scenario 9: subgraph-level insert rule overrides what the subgraph sends
    // by not propagating the subgraph header and inserting a fixed value instead
    #[ntex::test]
    async fn subgraph_pinned_cache_control_from_config() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("accounts") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            // subgraph sends private, but config inserts a pinned value instead
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
                            response:
                                - insert:
                                    name: cache-control
                                    value: "public, max-age=120"
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
            "expected pinned public, max-age=120 from insert rule but got: {cc}"
        );
    }

    // scenario 10: http execution error (non-200 status) from subgraph forces no-store, no-cache, must-revalidate
    #[ntex::test]
    async fn subgraph_execution_error_disables_caching_even_with_insert() {
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

        // the subgraph errors with a 500 and sends no cache-control at all.
        // we insert caching so the aggregator has an entry
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                headers:
                    all:
                        response:
                            - insert:
                                name: cache-control
                                value: "public, max-age=300"
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

    // scenario 11: global propagate with default + subgraph-level insert override
    //
    // - accounts sends no cache-control -> global default "public, max-age=180"
    //   is used
    // - products sends "public, max-age=300" but the subgraph insert rule pins
    //   it to "no-cache"
    // - "no-cache" poisons the merge, so the result is "no-store, no-cache"
    //   regardless of the default
    #[ntex::test]
    async fn global_default_with_subgraph_insert_override() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|req| {
                if req.path.contains("products") {
                    Some(ResponseLike::new(
                        StatusCode::OK,
                        None,
                        some_header_map! {
                            // products sends its own value; the insert rule replaces it
                            http::header::CACHE_CONTROL => "public, max-age=300"
                        },
                    ))
                } else {
                    // accounts sends nothing; the global default should fill in
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
                    all:
                        response:
                            - propagate:
                                named: cache-control
                                algorithm: append
                                default: "public, max-age=180"
                    subgraphs:
                        products:
                            response:
                                - insert:
                                    name: cache-control
                                    value: "no-cache"
                "#,
            )
            .build()
            .start()
            .await;

        // topProducts hits products; me hits accounts
        // use topProducts so products subgraph is definitely involved
        let res = router
            .send_graphql_request(
                r#"{ topProducts(first: 1) { upc name } me { name } }"#,
                None,
                None,
            )
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header");

        // products insert rule pins to no-cache, which poisons the merge
        // no-cache short-circuits the entire result regardless of the accounts default
        assert!(
            cc.contains("no-store") && cc.contains("no-cache"),
            "expected no-store, no-cache from poisoned merge but got: {cc}"
        );
        assert!(
            !cc.contains("max-age"),
            "expected max-age to be absent after poison but got: {cc}"
        );
    }

    // scenario 12: propagate default is used when the subgraph sends no cache-control
    #[ntex::test]
    async fn propagate_default_when_subgraph_sends_nothing() {
        // accounts sends no cache-control header at all
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
                        response:
                            - propagate:
                                named: cache-control
                                algorithm: append
                                default: "public, max-age=180"
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(r#"{ me { name } }"#, None, None)
            .await;

        assert_eq!(res.status(), 200);

        let cc = cache_control(&res).expect("expected cache-control header from default");
        assert!(
            cc.contains("public") && cc.contains("max-age=180"),
            "expected public, max-age=180 from propagate default but got: {cc}"
        );
    }

    #[ntex::test]
    async fn plugin_cache_control_is_kept_for_successful_queries() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = start_router_with_cacheable_plugin(&subgraphs).await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert_eq!(res.status(), 200);
        assert_eq!(cache_control(&res).as_deref(), Some("public, max-age=60"));
    }

    #[ntex::test]
    async fn plugin_cache_control_cannot_make_mutations_cacheable() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = start_router_with_cacheable_plugin(&subgraphs).await;

        let res = router
            .send_graphql_request(
                r#"mutation { oneofTest(input: { string: "dummy" }) { id } }"#,
                None,
                None,
            )
            .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            cache_control(&res).as_deref(),
            Some("no-store, no-cache, must-revalidate")
        );
    }

    #[ntex::test]
    async fn plugin_cache_control_is_no_store_when_end_hook_adds_errors() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = start_router_with_cacheable_plugin(&subgraphs).await;

        let res = router
            .send_graphql_request(
                "{ users { id } }",
                None,
                some_header_map! { http::header::HeaderName::from_static("x-add-error") => "1" },
            )
            .await;

        assert_eq!(res.status(), 200);
        assert_eq!(
            cache_control(&res).as_deref(),
            Some("no-store, no-cache, must-revalidate")
        );
    }
}
