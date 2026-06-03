#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};
    use hive_router::plugins::{
        hooks::{
            on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        },
        plugin_trait::{RouterPlugin, StartHookPayload},
    };
    use hive_router::{async_trait, http::StatusCode, ntex, sonic_rs, GraphQLError};

    #[derive(Default)]
    struct MultipleGraphQLErrorsPlugin;

    #[async_trait]
    impl RouterPlugin for MultipleGraphQLErrorsPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "forbid_anonymous_operations"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_graphql_params<'exec>(
            &'exec self,
            payload: OnGraphQLParamsStartHookPayload<'exec>,
        ) -> OnGraphQLParamsStartHookResult<'exec> {
            payload.end_with_graphql_errors(
                vec![
                    GraphQLError::from_message_and_code("First violation", "FIRST_VIOLATION"),
                    GraphQLError::from_message_and_code("Second violation", "SECOND_VIOLATION"),
                ],
                StatusCode::BAD_REQUEST,
            )
        }
    }

    #[ntex::test]
    async fn should_forbid_anonymous_operations() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/forbid_anonymous_operations/router.config.yaml")
            .register_plugin::<crate::plugin::ForbidAnonymousOperationsPlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ __schema { types { name } } }", None, None)
            .await;

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            res.json_body().await,
            sonic_rs::json!({
                "errors": [
                    {
                        "message": "Anonymous operations are not allowed",
                        "extensions": {
                            "code": "ANONYMOUS_OPERATION"
                        }
                    }
                ]
            })
        );
    }

    #[ntex::test]
    async fn should_return_multiple_graphql_errors() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/forbid_anonymous_operations/router.config.yaml")
            .register_plugin::<MultipleGraphQLErrorsPlugin>()
            .build()
            .start()
            .await;

        let res = router
            .send_graphql_request("{ users { id } }", None, None)
            .await;

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            res.json_body().await,
            sonic_rs::json!({
                "errors": [
                    {
                        "message": "First violation",
                        "extensions": {
                            "code": "FIRST_VIOLATION"
                        }
                    },
                    {
                        "message": "Second violation",
                        "extensions": {
                            "code": "SECOND_VIOLATION"
                        }
                    }
                ]
            })
        );
    }
}
