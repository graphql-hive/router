#[cfg(test)]
mod subgraph_apq_e2e_tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use http::{header::CONTENT_TYPE, HeaderMap, HeaderValue, StatusCode};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    use crate::testkit::{ClientResponseExt, RequestLike, ResponseLike, TestRouter, TestSubgraphs};

    fn json_response(status: StatusCode, body: Value) -> ResponseLike {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        ResponseLike::new(status, Some(body.to_string()), Some(headers))
    }

    fn body_json(request: &RequestLike) -> Value {
        serde_json::from_slice(request.body.as_ref().unwrap()).unwrap()
    }

    fn apq_hash(body: &Value) -> Option<&str> {
        body.pointer("/extensions/persistedQuery/sha256Hash")
            .and_then(Value::as_str)
    }

    fn sha256(query: &Value) -> String {
        hex::encode(Sha256::digest(query.as_str().unwrap().as_bytes()))
    }

    /// An APQ `accounts` subgraph. Known hashes get `{ users: [{ id: "from-apq" }] }`.
    async fn apq_subgraphs(supported: bool) -> TestSubgraphs<crate::testkit::Started> {
        apq_subgraphs_builder(supported).build().start().await
    }

    fn apq_subgraphs_builder(supported: bool) -> crate::testkit::TestSubgraphsBuilder {
        let known_hashes = Arc::new(Mutex::new(HashSet::<String>::new()));
        TestSubgraphs::builder().with_on_request(move |request| {
            if request.path != "/accounts" {
                return None;
            }
            let body = body_json(&request);
            let hash = apq_hash(&body)?.to_string();
            if !supported {
                // what a server without APQ says to a request with no query
                return Some(json_response(
                    StatusCode::BAD_REQUEST,
                    json!({ "errors": [{ "message": "Must provide query string.", "extensions": { "code": "BAD_REQUEST" } }] }),
                ));
            }
            let (status, code) = if body.get("query").is_some() {
                if hash != sha256(&body["query"]) {
                    (StatusCode::BAD_REQUEST, "PERSISTED_QUERY_HASH_MISMATCH")
                } else {
                    known_hashes.lock().unwrap().insert(hash);
                    return None;
                }
            } else if known_hashes.lock().unwrap().contains(&hash) {
                return Some(json_response(
                    StatusCode::OK,
                    json!({ "data": { "users": [{ "id": "from-apq" }] } }),
                ));
            } else {
                (StatusCode::NOT_FOUND, "PERSISTED_QUERY_NOT_FOUND")
            };
            Some(json_response(
                status,
                json!({ "errors": [{ "message": "x", "extensions": { "code": code } }] }),
            ))
        })
    }

    #[ntex::test]
    async fn sends_hash_then_query_then_hash() {
        let subgraphs = apq_subgraphs(true).await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        automatic_persisted_queries: true
                    subgraphs:
                        products:
                            automatic_persisted_queries: false
                "#,
            )
            .build()
            .start()
            .await;
        let query = "{ users { id } topProducts { upc } }";

        let res = router.send_graphql_request(query, None, None).await;
        let body = res.json_body_string_pretty().await;
        assert!(
            !body.contains("from-apq") && !body.contains("errors"),
            "{body}"
        );

        let requests = subgraphs.get_requests_log("accounts").unwrap();
        assert_eq!(requests.len(), 2);
        let hash_only = body_json(&requests[0]);
        let with_query = body_json(&requests[1]);
        assert!(hash_only.get("query").is_none(), "{hash_only}");
        assert_eq!(apq_hash(&hash_only), apq_hash(&with_query));
        assert_eq!(
            apq_hash(&with_query),
            Some(sha256(&with_query["query"]).as_str())
        );

        let products = body_json(&subgraphs.get_requests_log("products").unwrap()[0]);
        assert!(products.get("extensions").is_none(), "{products}");

        let res = router.send_graphql_request(query, None, None).await;
        let body = res.json_body_string_pretty().await;
        assert!(body.contains("from-apq"), "{body}");
        assert_eq!(subgraphs.get_requests_log("accounts").unwrap().len(), 3);
    }

    #[ntex::test]
    async fn turns_apq_off_when_not_supported() {
        let subgraphs = apq_subgraphs(false).await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        automatic_persisted_queries: true
                "#,
            )
            .build()
            .start()
            .await;

        for _ in 0..2 {
            let res = router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
            let body = res.json_body_string_pretty().await;
            assert!(
                body.contains("\"users\"") && !body.contains("errors"),
                "{body}"
            );
        }

        let requests = subgraphs.get_requests_log("accounts").unwrap();
        assert_eq!(requests.len(), 3);
        for request in &requests[1..] {
            assert!(body_json(request).get("extensions").is_none());
        }
    }

    #[ntex::test]
    async fn failures_that_also_fail_without_apq_leave_it_on() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|request| {
                (request.path == "/accounts").then(|| {
                    json_response(
                        StatusCode::UNAUTHORIZED,
                        json!({ "errors": [{ "message": "Unauthorized" }] }),
                    )
                })
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
                traffic_shaping:
                    all:
                        automatic_persisted_queries: true
                "#,
            )
            .build()
            .start()
            .await;

        for _ in 0..2 {
            router
                .send_graphql_request("{ users { id } }", None, None)
                .await;
        }

        // hash, plain, and the hash again: still on
        let sent_hash: Vec<_> = subgraphs
            .get_requests_log("accounts")
            .unwrap()
            .iter()
            .map(|request| apq_hash(&body_json(request)).is_some())
            .collect();
        assert_eq!(sent_hash, [true, false, true, false]);
    }

    static HOOK_RUNS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Default)]
    struct RenamingPlugin;

    #[hive_router::async_trait]
    impl hive_router::plugins::plugin_trait::RouterPlugin for RenamingPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_apq_renaming"
        }

        fn on_plugin_init(
            payload: hive_router::plugins::hooks::on_plugin_init::OnPluginInitPayload<Self>,
        ) -> hive_router::plugins::hooks::on_plugin_init::OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_subgraph_http_request<'exec>(
            &'exec self,
            mut payload: hive_router::plugins::hooks::on_subgraph_http_request::OnSubgraphHttpRequestHookPayload<'exec>,
        ) -> hive_router::plugins::hooks::on_subgraph_http_request::OnSubgraphHttpRequestHookResult<'exec>
        {
            use hive_router::plugins::plugin_trait::StartHookPayload;
            HOOK_RUNS.fetch_add(1, Ordering::Relaxed);
            payload.execution_request.operation_name = Some("Hooked".to_string());
            payload.proceed()
        }
    }

    #[ntex::test]
    async fn retry_ignores_what_hooks_changed() {
        let subgraphs = apq_subgraphs(true).await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        automatic_persisted_queries: true
                plugins:
                    test_apq_renaming:
                        enabled: true
                "#,
            )
            .register_plugin::<RenamingPlugin>()
            .build()
            .start()
            .await;

        router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert_eq!(HOOK_RUNS.load(Ordering::Relaxed), 2);
        let with_query = body_json(&subgraphs.get_requests_log("accounts").unwrap()[1]);
        assert!(with_query.get("operationName").is_none(), "{with_query}");
        assert_eq!(
            apq_hash(&with_query),
            Some(sha256(&with_query["query"]).as_str())
        );
    }

    const REWRITTEN: &str = "{users{id __typename}}";

    /// Swaps the query on `x-rewrite`, turns APQ off on `x-no-apq`.
    #[derive(Default)]
    struct QueryPlugin;

    #[hive_router::async_trait]
    impl hive_router::plugins::plugin_trait::RouterPlugin for QueryPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_apq_query"
        }

        fn on_plugin_init(
            payload: hive_router::plugins::hooks::on_plugin_init::OnPluginInitPayload<Self>,
        ) -> hive_router::plugins::hooks::on_plugin_init::OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_subgraph_execute<'exec>(
            &'exec self,
            mut payload: hive_router::plugins::hooks::on_subgraph_execute::OnSubgraphExecuteStartHookPayload<'exec>,
        ) -> hive_router::plugins::hooks::on_subgraph_execute::OnSubgraphExecuteStartHookResult<'exec>
        {
            use hive_router::plugins::plugin_trait::StartHookPayload;
            let headers = payload.router_http_request.headers;
            if headers.contains_key("x-rewrite") {
                payload.execution_request.query = REWRITTEN;
            }
            if headers.contains_key("x-no-apq") {
                payload.execution_request.apq = false;
            }
            payload.proceed()
        }
    }

    #[ntex::test]
    async fn plugin_query_changes_get_their_own_hash() {
        let subgraphs = apq_subgraphs(true).await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                    source: file
                    path: supergraph.graphql
                traffic_shaping:
                    all:
                        automatic_persisted_queries: true
                plugins:
                    test_apq_query:
                        enabled: true
                "#,
            )
            .register_plugin::<QueryPlugin>()
            .build()
            .start()
            .await;
        let send = |header: Option<&'static str>| {
            let headers = header.map(|name| {
                let mut headers = HeaderMap::new();
                headers.insert(name, HeaderValue::from_static("1"));
                headers
            });
            router.send_graphql_request("{ users { id } }", None, headers)
        };

        // original, rewritten and original again, against a cache the first one warmed
        for header in [None, Some("x-rewrite"), None] {
            let body = send(header).await.json_body_string_pretty().await;
            assert!(!body.contains("errors"), "{body}");
        }
        let requests = subgraphs.get_requests_log("accounts").unwrap();
        let hashes: Vec<_> = requests
            .iter()
            .map(|request| apq_hash(&body_json(request)).unwrap().to_string())
            .collect();
        let original = sha256(&body_json(&requests[1])["query"]);
        let rewritten = sha256(&json!(REWRITTEN));
        assert_eq!(
            hashes,
            [&original, &original, &rewritten, &rewritten, &original].map(String::as_str)
        );

        let body = send(Some("x-no-apq")).await.json_body_string_pretty().await;
        assert!(!body.contains("errors"), "{body}");
        let requests = subgraphs.get_requests_log("accounts").unwrap();
        assert_eq!(requests.len(), 6);
        assert!(body_json(&requests[5]).get("extensions").is_none());
    }

    #[ntex::test]
    async fn retry_gets_what_is_left_of_the_timeout() {
        let subgraphs = apq_subgraphs_builder(true)
            .with_path_delay("/accounts", std::time::Duration::from_millis(150))
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
                traffic_shaping:
                    all:
                        automatic_persisted_queries: true
                        request_timeout: 250ms
                "#,
            )
            .build()
            .start()
            .await;

        // a miss and a retry take 300ms, more than one timeout
        let body = router
            .send_graphql_request("{ users { id } }", None, None)
            .await
            .json_body_string_pretty()
            .await;
        assert!(body.contains("errors"), "{body}");
        assert_eq!(subgraphs.get_requests_log("accounts").unwrap().len(), 2);
    }
}
