#[cfg(test)]
mod subscription_plugin_hooks_e2e_tests {

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

    const INJECTED_HEADER_NAME: &str = "x-subscription-plugin-injected";
    const INJECTED_HEADER_VALUE: &str = "from-on-subgraph-execute";

    /// Plugin that injects a marker header on every subgraph request from
    /// `on_subgraph_execute`. The test asserts that the header reaches the
    /// subgraph for the subscription registration request, proving that the
    /// hook is invoked on the subscribe path.
    #[derive(Default)]
    struct InjectSubscriptionHeaderPlugin;

    #[async_trait]
    impl RouterPlugin for InjectSubscriptionHeaderPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_inject_subscription_header"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_subgraph_execute<'exec>(
            &'exec self,
            mut payload: OnSubgraphExecuteStartHookPayload<'exec>,
        ) -> OnSubgraphExecuteStartHookResult<'exec> {
            payload.execution_request.headers.insert(
                ::http::header::HeaderName::from_static(INJECTED_HEADER_NAME),
                ::http::header::HeaderValue::from_static(INJECTED_HEADER_VALUE),
            );
            payload.proceed()
        }
    }

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
                    test_inject_subscription_header:
                        enabled: true
                "#,
            )
            .register_plugin::<InjectSubscriptionHeaderPlugin>()
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

        // Drain the body to ensure the subscription registration actually
        // happens and the subgraph receives the request.
        let _body = res.body().await.unwrap();

        let reviews_requests = subgraphs
            .get_requests_log("reviews")
            .expect("`reviews` subgraph should have received the subscription registration");

        assert!(
            !reviews_requests.is_empty(),
            "`reviews` subgraph should have received at least one request"
        );

        let header_value = reviews_requests
            .iter()
            .find_map(|req| req.headers.get(INJECTED_HEADER_NAME))
            .expect("the injected header must be present on the subgraph request");

        assert_eq!(
            header_value, INJECTED_HEADER_VALUE,
            "the injected header value must match the value set by the plugin"
        );
    }

    /// Plugin that always short-circuits with `end_with_response`. We use this
    /// to verify that the explicit error surfaces to the caller on the
    /// subscribe path instead of being silently dropped.
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
    /// the dedicated error code is surfaced to the client.
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

        let body = res.body().await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();

        assert!(
            body_str.contains("SUBGRAPH_SUBSCRIBE_PLUGIN_END_WITH_RESPONSE_UNSUPPORTED"),
            "expected the dedicated error code to be emitted in the SSE stream, got: {body_str}"
        );
    }
}
