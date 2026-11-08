#[cfg(test)]
mod indexed_plugins_e2e_tests {
    use bytes::Bytes;
    use hive_router::{
        async_trait,
        executor::response::{graphql_error::GraphQLError, subgraph_response::SubgraphResponse},
        plugins::hooks::on_execute::{
            OnExecuteEndHookPayload, OnExecuteStartHookPayload, OnExecuteStartHookResult,
        },
        plugins::hooks::on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        plugins::hooks::on_subgraph_execute::{
            OnSubgraphExecuteEndHookPayload, OnSubgraphExecuteStartHookPayload,
            OnSubgraphExecuteStartHookResult,
        },
        plugins::plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
    };
    use sonic_rs::{json, JsonContainerTrait, JsonValueTrait};

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
            payload.data = Some(Bytes::from_static(
                br#"{"users":[{"id":"from-plugin","username":"seeded"}]}"#,
            ));
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

    /// Changes subgraph data before it's merged, through the subgraph end hook
    #[derive(Default)]
    struct SubgraphEditPlugin;

    #[async_trait]
    impl RouterPlugin for SubgraphEditPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_subgraph_edit"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_subgraph_execute<'exec>(
            &'exec self,
            payload: OnSubgraphExecuteStartHookPayload<'exec>,
        ) -> OnSubgraphExecuteStartHookResult<'exec> {
            payload.on_end(|mut payload: OnSubgraphExecuteEndHookPayload<'exec>| {
                payload.data.walk(|value| {
                    if value.is("User", "username") {
                        let shouted = value.as_str().map(|name| name.to_uppercase());
                        let _ = value.set(shouted);
                    } else if value.is("User", "id") && value.as_str().as_deref() == Some("2") {
                        // Non-Null: the user becomes null
                        value.set_null_with_error("user 2 is private");
                    }
                });
                payload.proceed()
            })
        }
    }

    /// Puts data that isn't JSON in before the plan runs: it's left out
    #[derive(Default)]
    struct BadSeedPlugin;

    #[async_trait]
    impl RouterPlugin for BadSeedPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_bad_seed"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_execute<'exec>(
            &'exec self,
            mut payload: OnExecuteStartHookPayload<'exec>,
        ) -> OnExecuteStartHookResult<'exec> {
            payload.data = Some(Bytes::from_static(b"not json"));
            payload.proceed()
        }
    }

    /// Puts `null` in before the plan runs: not an object, so it's left out too
    #[derive(Default)]
    struct NullSeedPlugin;

    #[async_trait]
    impl RouterPlugin for NullSeedPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_null_seed"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_execute<'exec>(
            &'exec self,
            mut payload: OnExecuteStartHookPayload<'exec>,
        ) -> OnExecuteStartHookResult<'exec> {
            payload.data = Some(Bytes::from_static(b"null"));
            payload.proceed()
        }
    }

    /// Answers instead of the subgraph, with a JSON body
    #[derive(Default)]
    struct SubgraphAnswerPlugin;

    #[async_trait]
    impl RouterPlugin for SubgraphAnswerPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_subgraph_answer"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_subgraph_execute<'exec>(
            &'exec self,
            payload: OnSubgraphExecuteStartHookPayload<'exec>,
        ) -> OnSubgraphExecuteStartHookResult<'exec> {
            let mut response = SubgraphResponse::from_json(Bytes::from_static(
                br#"{"data":{"users":[{"id":"9","username":"from-plugin"}]},"errors":[{"message":"answered by a plugin"}]}"#,
            ));
            // errors next to the body come after the body's
            response.errors = Some(vec![GraphQLError::from("set next to the body")]);
            payload.end_with_response(response)
        }
    }

    /// Reads the subgraph's extensions in the end hook and reports them as an error
    #[derive(Default)]
    struct SubgraphExtensionsPlugin;

    #[async_trait]
    impl RouterPlugin for SubgraphExtensionsPlugin {
        type Config = ();

        fn plugin_name() -> &'static str {
            "test_subgraph_extensions"
        }

        fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
            payload.initialize_plugin_with_defaults()
        }

        async fn on_subgraph_execute<'exec>(
            &'exec self,
            payload: OnSubgraphExecuteStartHookPayload<'exec>,
        ) -> OnSubgraphExecuteStartHookResult<'exec> {
            payload.on_end(|mut payload: OnSubgraphExecuteEndHookPayload<'exec>| {
                let foo = payload
                    .extensions()
                    .and_then(|extensions| extensions["foo"].as_str().map(str::to_string));
                let status = payload.status().map(|status| status.as_u16());
                payload.errors.push(GraphQLError::from(format!(
                    "saw {} with {status:?}",
                    foo.as_deref().unwrap_or("nothing")
                )));
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

    /// The response with the indexed pipeline off and on: they must be the same
    async fn both_pipelines(plugin: &str, extra_config: &str, query: &str) -> (String, String) {
        let mut out = Vec::new();
        for indexed in [false, true] {
            let subgraphs = TestSubgraphs::builder()
                .with_on_request(|req| {
                    // the root `users` query only: entity fetches reach the accounts subgraph
                    let entities = req
                        .body
                        .as_ref()
                        .is_some_and(|body| body.windows(9).any(|w| w == b"_entities"));
                    if req.path.contains("accounts") && !entities {
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
                      experimental_indexed_pipeline: {indexed}
                    plugins:
                      {plugin}:
                        enabled: true
                    {extra_config}
                    "#
                ))
                .register_plugin::<SeedPlugin>()
                .register_plugin::<EditPlugin>()
                .register_plugin::<SubgraphEditPlugin>()
                .register_plugin::<BadSeedPlugin>()
                .register_plugin::<NullSeedPlugin>()
                .register_plugin::<SubgraphAnswerPlugin>()
                .register_plugin::<SubgraphExtensionsPlugin>()
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
    async fn a_plugin_keeps_subgraph_extensions_and_its_own_data_on_the_indexed_pipeline() {
        let (off, on) = both_pipelines(
            "test_seed",
            "response_extensions:\n                      propagate:\n                        algorithm: last",
            "{ users { id username } }",
        )
        .await;
        assert_eq!(on, off, "indexed pipeline response differs");
        assert!(
            on.contains("from-accounts"),
            "subgraph extensions kept: {on}"
        );
    }

    #[ntex::test]
    async fn a_plugin_changes_the_data_the_same_way_on_both_pipelines() {
        let (off, on) = both_pipelines("test_edit", "", "{ users { id username } }").await;
        assert_eq!(on, off, "indexed pipeline response differs");
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

    #[ntex::test]
    async fn a_subgraph_hook_changes_the_data_the_same_way_on_both_pipelines() {
        // errors from a subgraph hook are the subgraph's: masked like its own unless turned off
        let (off, on) = both_pipelines(
            "test_subgraph_edit",
            "error_masking:\n                      enabled: false",
            "{ users { id username } }",
        )
        .await;
        assert_eq!(on, off, "indexed pipeline response differs");
        let body: sonic_rs::Value = sonic_rs::from_str(&on).expect("json");
        assert_eq!(
            body["data"],
            json!({ "users": [{ "id": "1", "username": "A" }, null] }),
            "username changed, user 2 nulled: {on}"
        );
        assert_eq!(
            body["errors"][0]["message"],
            json!("user 2 is private"),
            "{on}"
        );
        assert_eq!(body["errors"][0]["path"], json!(["users", 1, "id"]), "{on}");
    }

    #[ntex::test]
    async fn a_subgraph_hook_changes_entities_the_same_way_on_both_pipelines() {
        let (off, on) = both_pipelines(
            "test_subgraph_edit",
            "",
            "{ topProducts { upc reviews { author { username } } } }",
        )
        .await;
        assert_eq!(on, off, "indexed pipeline response differs");
        let body: sonic_rs::Value = sonic_rs::from_str(&on).expect("json");
        let mut usernames = Vec::new();
        for product in body["data"]["topProducts"]
            .as_array()
            .expect("products")
            .iter()
        {
            for review in product["reviews"].as_array().into_iter().flatten() {
                usernames.push(
                    review["author"]["username"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                );
            }
        }
        assert!(!usernames.is_empty(), "authors came from _entities: {on}");
        assert!(
            usernames
                .iter()
                .all(|name| !name.is_empty() && *name == name.to_uppercase()),
            "every author's username changed in the entity fetch: {on}"
        );
    }

    #[ntex::test]
    async fn data_a_plugin_sets_that_isnt_json_is_left_out_on_both_pipelines() {
        let (off, on) = both_pipelines("test_bad_seed", "", "{ users { id username } }").await;
        assert_eq!(on, off, "indexed pipeline response differs");
        let body: sonic_rs::Value = sonic_rs::from_str(&on).expect("json");
        assert_eq!(
            body["data"],
            json!({ "users": [{ "id": "1", "username": "a" }, { "id": "2", "username": "b" }] }),
            "the subgraph's data, nothing from the plugin: {on}"
        );
    }

    #[ntex::test]
    async fn null_data_a_plugin_sets_is_left_out_on_both_pipelines() {
        let (off, on) = both_pipelines("test_null_seed", "", "{ users { id username } }").await;
        assert_eq!(on, off, "indexed pipeline response differs");
        let body: sonic_rs::Value = sonic_rs::from_str(&on).expect("json");
        assert_eq!(
            body["data"],
            json!({ "users": [{ "id": "1", "username": "a" }, { "id": "2", "username": "b" }] }),
            "the subgraph's data, not null: {on}"
        );
    }

    #[ntex::test]
    async fn a_plugins_json_answer_is_read_like_a_subgraphs_on_both_pipelines() {
        let (off, on) = both_pipelines(
            "test_subgraph_answer",
            "error_masking:\n                      enabled: false",
            "{ users { id username } }",
        )
        .await;
        assert_eq!(on, off, "indexed pipeline response differs");
        let body: sonic_rs::Value = sonic_rs::from_str(&on).expect("json");
        assert_eq!(
            body["data"],
            json!({ "users": [{ "id": "9", "username": "from-plugin" }] }),
            "{on}"
        );
        assert_eq!(
            body["errors"][0]["message"],
            json!("answered by a plugin"),
            "{on}"
        );
        assert_eq!(
            body["errors"][1]["message"],
            json!("set next to the body"),
            "{on}"
        );
    }

    #[ntex::test]
    async fn a_subgraph_hook_reads_extensions_and_status_on_both_pipelines() {
        let (off, on) = both_pipelines(
            "test_subgraph_extensions",
            "error_masking:\n                      enabled: false",
            "{ users { id } }",
        )
        .await;
        assert_eq!(on, off, "indexed pipeline response differs");
        assert!(
            on.contains("saw from-accounts with Some(200)"),
            "the hook read the subgraph's extensions and status: {on}"
        );
    }
}
