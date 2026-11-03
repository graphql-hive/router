#[cfg(test)]
mod request_extensions_propagation_e2e_tests {
    use std::collections::HashMap;

    use futures::StreamExt;
    use hive_router::executor::executors::{
        graphql_transport_ws::SubscribePayload, websocket_client::WsClient,
    };
    use sonic_rs::{json, JsonValueTrait, Value};

    use crate::testkit::{some_header_map, RequestLike, Started, TestRouter, TestSubgraphs};

    fn sent_extensions(request: &RequestLike) -> Option<Value> {
        let body: Value =
            sonic_rs::from_slice(request.body.as_ref().expect("expected request body"))
                .expect("expected valid JSON body");
        body.get("extensions").cloned()
    }

    fn sent_extensions_of_all(
        subgraphs: &TestSubgraphs<Started>,
        subgraph: &str,
    ) -> Vec<Option<Value>> {
        subgraphs
            .get_requests_log(subgraph)
            .unwrap_or_else(|| panic!("expected requests sent to {subgraph} subgraph"))
            .iter()
            .map(sent_extensions)
            .collect()
    }

    #[ntex::test]
    async fn should_not_propagate_extensions_by_default() {
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
            .send_post_request(
                router.graphql_path(),
                json!({
                    "query": "{ topProducts { upc } }",
                    "extensions": { "tenant": "acme" }
                }),
                None,
            )
            .await;
        assert!(res.status().is_success(), "expected 200 OK");

        assert_eq!(sent_extensions_of_all(&subgraphs, "products"), vec![None]);
    }

    #[ntex::test]
    async fn should_propagate_allowed_keys_to_root_and_entity_fetches() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                request_extensions:
                  propagate:
                    mode: allow
                    keys: [tenant]
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_post_request(
                router.graphql_path(),
                json!({
                    "query": "{ topProducts { upc reviews { id } } }",
                    "extensions": { "tenant": "acme", "debug": true }
                }),
                None,
            )
            .await;
        assert!(res.status().is_success(), "expected 200 OK");

        let expected = Some(json!({ "tenant": "acme" }));
        assert_eq!(
            sent_extensions_of_all(&subgraphs, "products"),
            vec![expected.clone()]
        );
        assert_eq!(
            sent_extensions_of_all(&subgraphs, "reviews"),
            vec![expected]
        );
    }

    #[ntex::test]
    async fn should_propagate_all_but_denied_and_router_owned_keys() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                request_extensions:
                  propagate:
                    mode: deny
                    keys: [debug]
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_post_request(
                router.graphql_path(),
                json!({
                    "query": "{ topProducts { upc } }",
                    "extensions": {
                        "tenant": "acme",
                        "clientInfo": { "name": "web" },
                        "debug": true,
                        "persistedQuery": { "version": 1, "sha256Hash": "abc" },
                        "headers": { "authorization": "Bearer secret" },
                        "subscription": { "callbackUrl": "http://example.com" }
                    }
                }),
                None,
            )
            .await;
        assert!(res.status().is_success(), "expected 200 OK");

        assert_eq!(
            sent_extensions_of_all(&subgraphs, "products"),
            vec![Some(
                json!({ "tenant": "acme", "clientInfo": { "name": "web" } })
            )]
        );
    }

    #[ntex::test]
    async fn should_not_propagate_client_extensions_as_jwt_claims() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                jwt:
                  enabled: true
                  require_authentication: false
                  forward_claims_to_upstream_extensions:
                    enabled: true
                    field_name: jwt
                  jwks_providers:
                    - source: file
                      path: jwks.rsa512.json
                request_extensions:
                  propagate:
                    mode: deny
                    keys: []
                "#,
            )
            .build()
            .start()
            .await;

        // Not authenticated, so the router has no claims to forward,
        // and must not forward the client's own `jwt` extension instead.
        let res = router
            .send_post_request(
                router.graphql_path(),
                json!({
                    "query": "{ topProducts { upc } }",
                    "extensions": { "tenant": "acme", "jwt": { "sub": "admin" } }
                }),
                None,
            )
            .await;
        assert!(res.status().is_success(), "expected 200 OK");

        assert_eq!(
            sent_extensions_of_all(&subgraphs, "products"),
            vec![Some(json!({ "tenant": "acme" }))]
        );
    }

    #[ntex::test]
    async fn should_propagate_to_subscription_and_its_entity_fetches() {
        let subgraphs = TestSubgraphs::builder().build().start().await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                subscriptions:
                  enabled: true
                request_extensions:
                  propagate:
                    mode: allow
                    keys: [tenant]
                "#,
            )
            .build()
            .start()
            .await;

        let res = router
            .send_post_request(
                router.graphql_path(),
                json!({
                    "query": "subscription { reviewAdded(intervalInMs: 0) { id product { name } } }",
                    "extensions": { "tenant": "acme", "debug": true }
                }),
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;
        assert!(res.status().is_success(), "expected 200 OK");
        res.body().await.expect("expected subscription stream");

        let expected = Some(json!({ "tenant": "acme" }));
        assert_eq!(
            sent_extensions_of_all(&subgraphs, "reviews"),
            vec![expected.clone()]
        );

        let products_extensions = sent_extensions_of_all(&subgraphs, "products");
        assert!(
            !products_extensions.is_empty(),
            "expected entity fetches to the products subgraph"
        );
        assert!(
            products_extensions
                .iter()
                .all(|extensions| extensions == &expected),
            "expected every entity fetch to carry the propagated extensions, got {products_extensions:?}"
        );
    }

    #[ntex::test]
    async fn should_not_propagate_websocket_operation_headers() {
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
                    request:
                      - propagate:
                          named: x-context
                websocket:
                  enabled: true
                  headers:
                    source: operation
                request_extensions:
                  propagate:
                    mode: deny
                    keys: []
                "#,
            )
            .build()
            .start()
            .await;

        let mut client = WsClient::new(router.ws().await)
            .init(None)
            .await
            .expect("Failed to init WsClient");

        let subscribe_payload = SubscribePayload {
            query: "{ topProducts { upc } }".into(),
            extensions: Some(HashMap::from([
                ("headers".to_string(), json!({ "x-context": "from-ws" })),
                ("tenant".to_string(), json!("acme")),
            ])),
            ..Default::default()
        };

        let mut stream = client
            .subscribe(subscribe_payload, None)
            .await
            .expect("Failed to subscribe")
            .map(|response| response.expect("WebSocket response failed"));
        stream.next().await.expect("Expected a response");

        let products_requests = subgraphs
            .get_requests_log("products")
            .expect("expected requests sent to products subgraph");
        let request = products_requests
            .last()
            .expect("expected at least one request to products subgraph");

        assert_eq!(
            request
                .headers
                .get("x-context")
                .expect("expected the operation header to be propagated as a header"),
            "from-ws"
        );
        assert_eq!(sent_extensions(request), Some(json!({ "tenant": "acme" })));
    }
}
