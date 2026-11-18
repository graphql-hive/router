#[cfg(test)]
mod subgraph_apq_e2e_tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use hive_router::plugins::hooks::{
        on_subgraph_execute::{
            OnSubgraphExecuteStartHookPayload, OnSubgraphExecuteStartHookResult,
        },
        on_subgraph_http_request::{
            OnSubgraphHttpRequestHookPayload, OnSubgraphHttpRequestHookResult,
        },
    };
    use http::{header::CONTENT_TYPE, HeaderMap, HeaderValue, StatusCode};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    use crate::testkit::{
        ClientResponseExt, RequestLike, ResponseLike, Started, TestRouter, TestSubgraphs,
    };

    const APQ_ON: &str = "{ all: { automatic_persisted_queries: true } }";

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

    fn sent_hash(subgraphs: &TestSubgraphs<Started>, subgraph: &str) -> Vec<bool> {
        let requests = subgraphs.get_requests_log(subgraph).unwrap();
        requests
            .iter()
            .map(|request| apq_hash(&body_json(request)).is_some())
            .collect()
    }

    fn x_test(mode: &'static str) -> Option<HeaderMap> {
        Some(HeaderMap::from_iter([(
            "x-test".parse().unwrap(),
            HeaderValue::from_static(mode),
        )]))
    }

    /// `traffic_shaping` is YAML. With `plugin`, the client's `x-test` header drives [`TestPlugin`].
    async fn start_router(
        subgraphs: &TestSubgraphs<Started>,
        traffic_shaping: &str,
        plugin: bool,
    ) -> TestRouter<Started> {
        let mut config = format!(
            "supergraph: {{ source: file, path: supergraph.graphql }}\ntraffic_shaping: {traffic_shaping}\n"
        );
        let mut router = TestRouter::builder().with_subgraphs(subgraphs);
        if plugin {
            config += "headers: { all: { request: [{ propagate: { named: x-test } }] } }\n";
            config += "plugins: { test_apq: { enabled: true } }\n";
            router = router.register_plugin::<TestPlugin>();
        }
        router.inline_config(config).build().start().await
    }

    /// An APQ `accounts` subgraph. Known hashes get `{ users: [{ id: "from-apq" }] }`.
    async fn apq_subgraphs() -> TestSubgraphs<Started> {
        apq_subgraphs_builder().build().start().await
    }

    fn apq_subgraphs_builder() -> crate::testkit::TestSubgraphsBuilder {
        let known_hashes = Arc::new(Mutex::new(HashSet::<String>::new()));
        TestSubgraphs::builder().with_on_request(move |request| {
            if request.path != "/accounts" {
                return None;
            }
            let body = body_json(&request);
            let hash = apq_hash(&body)?.to_string();
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

    static HTTP_HOOK_RUNS: AtomicUsize = AtomicUsize::new(0);
    const REWRITTEN: &str = "{users{id __typename}}";

    fn denied() -> hive_router::SubgraphHttpResponse {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        hive_router::SubgraphHttpResponse {
            status: StatusCode::FORBIDDEN,
            headers: Arc::new(headers),
            body: r#"{"errors":[{"message":"Denied"}]}"#.into(),
        }
    }

    fn mode(headers: &HeaderMap) -> String {
        let mode = headers.get("x-test").and_then(|value| value.to_str().ok());
        mode.unwrap_or_default().to_string()
    }

    #[derive(Default)]
    struct TestPlugin;

    #[hive_router::async_trait]
    impl hive_router::plugins::plugin_trait::RouterPlugin for TestPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_apq"
        }

        fn on_plugin_init(
            payload: hive_router::plugins::hooks::on_plugin_init::OnPluginInitPayload<Self>,
        ) -> hive_router::plugins::hooks::on_plugin_init::OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_subgraph_execute<'exec>(
            &'exec self,
            mut payload: OnSubgraphExecuteStartHookPayload<'exec>,
        ) -> OnSubgraphExecuteStartHookResult<'exec> {
            use hive_router::plugins::plugin_trait::StartHookPayload;
            if mode(&payload.execution_request.headers) == "rewrite" {
                payload.execution_request.query = REWRITTEN;
            }
            payload.proceed()
        }

        async fn on_subgraph_http_request<'exec>(
            &'exec self,
            mut payload: OnSubgraphHttpRequestHookPayload<'exec>,
        ) -> OnSubgraphHttpRequestHookResult<'exec> {
            use hive_router::plugins::plugin_trait::{EndHookPayload, StartHookPayload};
            HTTP_HOOK_RUNS.fetch_add(1, Ordering::Relaxed);
            match mode(&payload.execution_request.headers).as_str() {
                "deny" => payload.end_with_response(denied()),
                "replace" => payload.on_end(|payload| payload.end_with_response(denied())),
                "rename" => {
                    payload.execution_request.operation_name = Some("Hooked".to_string());
                    payload.proceed()
                }
                _ => payload.proceed(),
            }
        }
    }

    #[ntex::test]
    async fn sends_hash_then_query_then_hash() {
        let subgraphs = apq_subgraphs().await;
        let router = start_router(
            &subgraphs,
            "{ all: { automatic_persisted_queries: true }, subgraphs: { products: { automatic_persisted_queries: false } } }",
            false,
        )
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
    async fn turns_apq_off_only_for_subgraphs_without_it() {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(|request| {
                let hash_only = body_json(&request).get("query").is_none();
                match request.path.as_str() {
                    // a server without APQ
                    "/accounts" if hash_only => Some(json_response(
                        StatusCode::BAD_REQUEST,
                        json!({ "errors": [{ "message": "Must provide query string." }] }),
                    )),
                    // fails with or without APQ
                    "/products" => Some(json_response(
                        StatusCode::UNAUTHORIZED,
                        json!({ "errors": [{ "message": "Unauthorized" }] }),
                    )),
                    _ => None,
                }
            })
            .build()
            .start()
            .await;
        let router = start_router(&subgraphs, APQ_ON, false).await;

        for _ in 0..2 {
            let body = router
                .send_graphql_request("{ users { id } }", None, None)
                .await
                .json_body_string_pretty()
                .await;
            assert!(
                body.contains("\"users\"") && !body.contains("errors"),
                "{body}"
            );
            router
                .send_graphql_request("{ topProducts { upc } }", None, None)
                .await;
        }
        router
            .send_graphql_request("mutation { reentryTest { ok } }", None, None)
            .await;

        assert_eq!(sent_hash(&subgraphs, "accounts"), [true, false, false]);
        // queries retry plainly and keep APQ on, the mutation doesn't retry
        assert_eq!(
            sent_hash(&subgraphs, "products"),
            [true, false, true, false, true]
        );
    }

    #[ntex::test]
    async fn retries_ignore_plugin_answers_and_changes() {
        let subgraphs = apq_subgraphs().await;
        let router = start_router(&subgraphs, APQ_ON, true).await;

        for mode in ["deny", "replace", "rename"] {
            router
                .send_graphql_request("{ users { id } }", None, x_test(mode))
                .await;
        }

        // deny and replace once each, rename twice
        assert_eq!(HTTP_HOOK_RUNS.load(Ordering::Relaxed), 4);
        let requests = subgraphs.get_requests_log("accounts").unwrap();
        assert_eq!(requests.len(), 3);
        let with_query = body_json(&requests[2]);
        assert!(with_query.get("operationName").is_none(), "{with_query}");
        assert_eq!(
            apq_hash(&with_query),
            Some(sha256(&with_query["query"]).as_str())
        );

        // the copy kept for the retry has no representations, the retry gets them back
        let body = router
            .send_graphql_request(
                "{ topProducts { reviews { author { name } } } }",
                None,
                None,
            )
            .await
            .json_body_string_pretty()
            .await;
        assert!(
            body.contains("\"name\"") && !body.contains("errors"),
            "{body}"
        );
        let with_query = body_json(&subgraphs.get_requests_log("accounts").unwrap()[4]);
        assert!(
            with_query["variables"]["representations"].is_array(),
            "{with_query}"
        );
    }

    #[ntex::test]
    async fn plugin_query_changes_get_their_own_hash() {
        let subgraphs = apq_subgraphs().await;
        let router = start_router(&subgraphs, APQ_ON, true).await;
        let send = |header| router.send_graphql_request("{ users { id } }", None, header);

        // original, rewritten and original again, against a cache the first one warmed
        for header in [None, x_test("rewrite"), None] {
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
    }

    #[ntex::test]
    async fn retry_gets_what_is_left_of_the_timeout() {
        let subgraphs = apq_subgraphs_builder()
            .with_path_delay("/accounts", std::time::Duration::from_millis(150))
            .build()
            .start()
            .await;
        let router = start_router(
            &subgraphs,
            "{ all: { automatic_persisted_queries: true, request_timeout: 250ms } }",
            false,
        )
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
