#[cfg(test)]
mod slot_plugins_e2e_tests {
    use bytes::Bytes;
    use hive_router::{
        async_trait,
        executor::response::value::Value,
        plugins::hooks::on_execute::{
            OnExecuteEndHookPayload, OnExecuteStartHookPayload, OnExecuteStartHookResult,
        },
        plugins::hooks::on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        plugins::plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
    };
    use sonic_rs::json;

    use crate::testkit::{ClientResponseExt, ResponseLike, TestRouter, TestSubgraphs};

    /// Puts data in before the plan runs, which the fetches then merge into
    #[derive(Default)]
    struct SeedPlugin;

    #[async_trait]
    impl RouterPlugin for SeedPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_seed"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_execute<'exec>(
            &'exec self,
            mut payload: OnExecuteStartHookPayload<'exec>,
        ) -> OnExecuteStartHookResult<'exec> {
            payload.data = Value::Object(vec![(
                "users",
                Value::Array(vec![Value::Object(vec![
                    ("id", Value::String("from-plugin".into())),
                    ("username", Value::String("seeded".into())),
                ])]),
            )]);
            payload.proceed()
        }
    }

    /// Changes the data the client gets, through the data API
    #[derive(Default)]
    struct EditPlugin;

    #[async_trait]
    impl RouterPlugin for EditPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_edit"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_execute<'exec>(
            &'exec self,
            payload: OnExecuteStartHookPayload<'exec>,
        ) -> OnExecuteStartHookResult<'exec> {
            payload.on_end(|mut payload: OnExecuteEndHookPayload<'exec>| {
                let mut outcomes = Vec::new();
                payload.data.walk(|value| {
                    if value.is("User", "username") {
                        let shouted = value.as_str().map(|name| name.to_uppercase());
                        outcomes.push(match value.set(shouted) {
                            Ok(()) => "ok".to_string(),
                            Err(error) => error.to_string(),
                        });
                        outcomes.push(match value.set(1) {
                            Ok(()) => "ok".to_string(),
                            Err(error) => error.to_string(),
                        });
                    } else if value.is("User", "id") && value.as_str().as_deref() == Some("2") {
                        // Non-Null: the user becomes null
                        value.set_null_with_error("user 2 is private");
                    }
                });
                payload.add_extension("edits", outcomes);
                payload.proceed()
            })
        }
    }

    fn json_response(body: sonic_rs::Value) -> ResponseLike {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        ResponseLike {
            status: axum::http::StatusCode::OK,
            headers,
            body: Some(Bytes::from(body.to_string())),
        }
    }

    /// The response with the slot pipeline off and on: they must be the same
    async fn both_pipelines(plugin: &str, extra_config: &str, query: &str) -> (String, String) {
        let mut out = Vec::new();
        for slots in [false, true] {
            let subgraphs = TestSubgraphs::builder()
                .with_on_request(|req| {
                    if req.path.contains("accounts") {
                        return Some(json_response(json!({
                            "data": { "users": [{ "id": "1", "username": "a" }, { "id": "2", "username": "b" }] },
                            "extensions": { "foo": "from-accounts" }
                        })));
                    }
                    None
                })
                .build()
                .start()
                .await;
            let router = TestRouter::builder()
                .with_subgraphs(&subgraphs)
                .inline_config(format!(
                    r#"
                    supergraph:
                      source: file
                      path: supergraph.graphql
                    query_planner:
                      experimental_slot_pipeline: {slots}
                    plugins:
                      {plugin}:
                        enabled: true
                    {extra_config}
                    "#
                ))
                .register_plugin::<SeedPlugin>()
                .register_plugin::<EditPlugin>()
                .build()
                .start()
                .await;
            let res = router.send_graphql_request(query, None, None).await;
            assert!(res.status().is_success(), "expected 200 OK");
            out.push(res.json_body_string_pretty().await);
        }
        let on = out.pop().expect("two runs");
        let off = out.pop().expect("two runs");
        (off, on)
    }

    #[ntex::test]
    async fn a_plugin_keeps_subgraph_extensions_and_its_own_data_on_slots() {
        let (off, on) = both_pipelines(
            "test_seed",
            "response_extensions:\n                      propagate:\n                        algorithm: last",
            "{ users { id username } }",
        )
        .await;
        assert_eq!(on, off, "slot pipeline response differs");
        assert!(
            on.contains("from-accounts"),
            "subgraph extensions kept: {on}"
        );
    }

    #[ntex::test]
    async fn a_plugin_changes_the_data_the_same_way_on_both_pipelines() {
        let (off, on) = both_pipelines("test_edit", "", "{ users { id username } }").await;
        assert_eq!(on, off, "slot pipeline response differs");
        let body: sonic_rs::Value = sonic_rs::from_str(&on).expect("json");
        assert_eq!(
            body["data"],
            json!({ "users": [{ "id": "1", "username": "A" }, null] }),
            "username changed, user 2 nulled: {on}"
        );
        assert_eq!(
            body["errors"],
            json!([{ "message": "user 2 is private", "path": ["users", 1, "id"] }]),
            "the error is at the nulled id: {on}"
        );
        assert!(
            on.contains("the value isn't of the field's type"),
            "a value of another type is refused: {on}"
        );
    }
}
