#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, TestRouterBuilder, TestSubgraphsBuilder};
    use hive_router::{http::StatusCode, ntex, sonic_rs};

    #[ntex::test]
    async fn should_forbid_anonymous_operations() {
        let subgraphs = TestSubgraphsBuilder::new().build().start().await;

        let router = TestRouterBuilder::new()
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
}
