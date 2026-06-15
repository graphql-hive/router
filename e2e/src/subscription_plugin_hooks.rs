#[cfg(test)]
mod subscription_plugin_hooks_e2e_tests {

    use std::time::Duration;

    use crate::testkit::{some_header_map, TestRouter, TestSubgraphs};
    use hive_router::{
        async_trait,
        plugins::hooks::on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        plugins::hooks::on_subgraph_execute::{
            OnSubgraphExecuteStartHookPayload, OnSubgraphExecuteStartHookResult,
        },
        plugins::plugin_trait::{RouterPlugin, StartHookPayload},
        GraphQLError,
    };
    use ntex::http;

    const FIRST_HEADER_NAME: &str = "x-subscription-plugin-first";
    const FIRST_HEADER_VALUE: &str = "from-first-plugin";
    const SECOND_HEADER_NAME: &str = "x-subscription-plugin-second";
    const SECOND_HEADER_VALUE: &str = "from-second-plugin";

    /// Drains the response body with a hard timeout so the test fails fast
    /// instead of hanging if the subgraph stub ever changes its emit cadence.
    async fn drain_body_with_timeout(res: ntex::client::ClientResponse) -> Vec<u8> {
        ntex::time::timeout(Duration::from_secs(5), res.body())
            .await
            .expect("response body should drain within 5s")
            .expect("response body should be readable")
            .to_vec()
    }

    macro_rules! define_header_plugin {
        ($plugin:ident, $name:literal, $header_name:expr, $header_value:expr) => {
            #[derive(Default)]
            struct $plugin;

            #[async_trait]
            impl RouterPlugin for $plugin {
                type Config = ();

                fn plugin_name() -> &'static str {
                    $name
                }

                fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
                    payload.initialize_plugin_with_defaults()
                }

                async fn on_subgraph_execute<'exec>(
                    &'exec self,
                    mut payload: OnSubgraphExecuteStartHookPayload<'exec>,
                ) -> OnSubgraphExecuteStartHookResult<'exec> {
                    payload.execution_request.headers.insert(
                        ::http::header::HeaderName::from_static($header_name),
                        ::http::header::HeaderValue::from_static($header_value),
                    );
                    payload.proceed()
                }
            }
        };
    }

    define_header_plugin!(
        InjectFirstHeaderPlugin,
        "test_inject_first_header",
        FIRST_HEADER_NAME,
        FIRST_HEADER_VALUE
    );
    define_header_plugin!(
        InjectSecondHeaderPlugin,
        "test_inject_second_header",
        SECOND_HEADER_NAME,
        SECOND_HEADER_VALUE
    );

    /// Subscription via SSE — verifies that `on_subgraph_execute` is invoked
    /// for the subscribe-registration request and that header mutations from
    /// the plugin reach the subgraph.
    #[ntex::test]
    async fn on_subgraph_execute_invoked_on_subscribe_sse() {
        let subgraphs = TestSubgraphs::builder()
            .with_http_streaming_subscriptions_protocol(
                subgraphs::HTTPStreamingSubscriptionProtocol::SseOnly,
            )
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
                subscriptions:
                    enabled: true
                plugins:
                    test_inject_first_header:
                        enabled: true
                "#,
            )
            .register_plugin::<InjectFirstHeaderPlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        product {
                            upc
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");
        let _body = drain_body_with_timeout(res).await;

        let reviews_requests = subgraphs
            .get_requests_log("reviews")
            .expect("`reviews` subgraph should have received the subscription registration");

        assert!(
            !reviews_requests.is_empty(),
            "`reviews` subgraph should have received at least one request"
        );

        let header_value = reviews_requests
            .iter()
            .find_map(|req| req.headers.get(FIRST_HEADER_NAME))
            .expect("the injected header must be present on the subgraph request");

        assert_eq!(
            header_value, FIRST_HEADER_VALUE,
            "the injected header value must match the value set by the plugin"
        );
    }

    /// Two plugins both mutate `execution_request.headers` from
    /// `on_subgraph_execute`. The subgraph should observe both header
    /// mutations, proving the plugin loop iterates fully on the subscribe
    /// path (not just stops after the first plugin).
    #[ntex::test]
    async fn on_subgraph_execute_chains_multiple_plugins_on_subscribe() {
        let subgraphs = TestSubgraphs::builder()
            .with_http_streaming_subscriptions_protocol(
                subgraphs::HTTPStreamingSubscriptionProtocol::SseOnly,
            )
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
                subscriptions:
                    enabled: true
                plugins:
                    test_inject_first_header:
                        enabled: true
                    test_inject_second_header:
                        enabled: true
                "#,
            )
            .register_plugin::<InjectFirstHeaderPlugin>()
            .register_plugin::<InjectSecondHeaderPlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        product {
                            upc
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");
        let _body = drain_body_with_timeout(res).await;

        let reviews_requests = subgraphs
            .get_requests_log("reviews")
            .expect("`reviews` subgraph should have received the subscription registration");

        let registration = reviews_requests
            .iter()
            .find(|req| req.headers.contains_key(FIRST_HEADER_NAME))
            .expect("at least one request should carry the first plugin's header");

        assert_eq!(
            registration.headers.get(FIRST_HEADER_NAME),
            Some(&::http::HeaderValue::from_static(FIRST_HEADER_VALUE)),
            "first plugin's header must be present"
        );
        assert_eq!(
            registration.headers.get(SECOND_HEADER_NAME),
            Some(&::http::HeaderValue::from_static(SECOND_HEADER_VALUE)),
            "second plugin's header must also be present (plugin loop iterates fully)"
        );
    }

    /// Plugin that short-circuits with `end_with_response` to drive the
    /// dedicated subscribe-path error. The status code/error code attached
    /// here intentionally never reach the client — the assertions below
    /// confirm we surface `SUBGRAPH_SUBSCRIBE_PLUGIN_HOOK_UNSUPPORTED`
    /// rather than the plugin's response.
    #[derive(Default)]
    struct EndWithResponsePlugin;

    #[async_trait]
    impl RouterPlugin for EndWithResponsePlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_end_with_response_on_subscribe"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_subgraph_execute<'exec>(
            &'exec self,
            payload: OnSubgraphExecuteStartHookPayload<'exec>,
        ) -> OnSubgraphExecuteStartHookResult<'exec> {
            payload.end_with_graphql_error(
                GraphQLError::from_message_and_code(
                    "short-circuit from plugin",
                    "TEST_PLUGIN_SHORT_CIRCUIT",
                ),
                http::StatusCode::INTERNAL_SERVER_ERROR,
            )
        }
    }

    /// `end_with_response` on the subscribe path is not yet supported. Confirm
    /// the dedicated error code is surfaced to the client and the plugin's
    /// own response payload never reaches it.
    #[ntex::test]
    async fn on_subgraph_execute_end_with_response_unsupported_on_subscribe() {
        let subgraphs = TestSubgraphs::builder()
            .with_http_streaming_subscriptions_protocol(
                subgraphs::HTTPStreamingSubscriptionProtocol::SseOnly,
            )
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
                subscriptions:
                    enabled: true
                plugins:
                    test_end_with_response_on_subscribe:
                        enabled: true
                "#,
            )
            .register_plugin::<EndWithResponsePlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request(
                r#"
                subscription {
                    reviewAdded(intervalInMs: 0) {
                        product {
                            upc
                        }
                    }
                }
                "#,
                None,
                some_header_map! {
                    http::header::ACCEPT => "text/event-stream"
                },
            )
            .await;

        assert_eq!(res.status(), 200, "Expected 200 OK");
        let body = drain_body_with_timeout(res).await;
        let body_str = std::str::from_utf8(&body).unwrap();

        assert!(
            body_str.contains("SUBGRAPH_SUBSCRIBE_PLUGIN_HOOK_UNSUPPORTED"),
            "expected the dedicated error code to be emitted in the SSE stream, got: {body_str}"
        );
        assert!(
            !body_str.contains("TEST_PLUGIN_SHORT_CIRCUIT"),
            "plugin's own response payload must NOT reach the client, got: {body_str}"
        );
    }
}
