use hive_router::{
    async_trait,
    plugins::hooks::on_graphql_params::{
        OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult,
    },
    plugins::hooks::on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
    plugins::hooks::on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
    plugins::plugin_trait::{RouterPlugin, StartHookPayload},
};
use sonic_rs::json;

use super::shared::{assert_error_code, assert_resolves_successfully, write_manifest};
use crate::testkit::{coprocessor::TestCoprocessor, TestRouter, TestSubgraphs};

struct TestSkipEnforcementPlugin {
    skip_enforcement: bool,
}

#[async_trait]
impl RouterPlugin for TestSkipEnforcementPlugin {
    type Config = bool;

    fn plugin_name() -> &'static str {
        "test_skip_enforcement"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        payload.initialize_plugin(Self {
            skip_enforcement: config,
        })
    }

    fn on_http_request<'req>(
        &self,
        payload: OnHttpRequestHookPayload<'req>,
    ) -> OnHttpRequestHookResult<'req> {
        payload
            .request_context
            .write()
            .unwrap()
            .persisted_documents()
            .set_skip_enforcement(self.skip_enforcement);
        payload.proceed()
    }
}

#[derive(Default)]
struct TestSkipEnforcementOnGraphqlParamsPlugin;

#[async_trait]
impl RouterPlugin for TestSkipEnforcementOnGraphqlParamsPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "test_skip_enforcement_on_graphql_params"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }

    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        payload
            .request_context
            .write()
            .unwrap()
            .persisted_documents()
            .set_skip_enforcement(true);
        payload.proceed()
    }
}

#[ntex::test]
async fn plugin_bypass_via_on_http_request() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                plugins:
                  test_skip_enforcement:
                    enabled: true
                    config: true
                "#,
            manifest.path().display(),
        ))
        // Plugin sets skip_enforcement to true on HTTP request
        .register_plugin::<TestSkipEnforcementPlugin>()
        .build()
        .start()
        .await;

    // We send no document id
    let response = router
        .send_graphql_request("{ topProducts { name } }", None, None)
        .await;

    // But it should still resolve successfully, because skip_enforcement is true
    assert_resolves_successfully(response).await;
}

#[ntex::test]
async fn plugin_bypass_via_on_graphql_params() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                plugins:
                  test_skip_enforcement_on_graphql_params:
                    enabled: true
                    config: true
                "#,
            manifest.path().display(),
        ))
        // Plugin sets skip_enforcement to true via on_graphql_params
        .register_plugin::<TestSkipEnforcementOnGraphqlParamsPlugin>()
        .build()
        .start()
        .await;

    // We send no document id
    let response = router
        .send_graphql_request("{ topProducts { name } }", None, None)
        .await;

    // But it should still resolve successfully, because skip_enforcement is true
    assert_resolves_successfully(response).await;
}

#[ntex::test]
async fn skip_enforcement_false_still_enforces() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                plugins:
                  test_skip_enforcement:
                    enabled: true
                    config: false
                "#,
            manifest.path().display(),
        ))
        // Plugin sets skip_enforcement to false via on_http_request
        .register_plugin::<TestSkipEnforcementPlugin>()
        .build()
        .start()
        .await;

    // We send no document id
    let response = router
        .send_graphql_request("{ topProducts { name } }", None, None)
        .await;

    // Request should fail as it lacks a document id
    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
}

#[ntex::test]
async fn skip_enforcement_with_valid_document_id() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                plugins:
                  test_skip_enforcement:
                    enabled: true
                    config: true
                "#,
            manifest.path().display(),
        ))
        // Plugin sets skip_enforcement to true
        .register_plugin::<TestSkipEnforcementPlugin>()
        .build()
        .start()
        .await;

    // We send the ID
    let response = router
        .send_post_request(
            "/graphql",
            json!({
                "documentId": "sha256:abc123"
            }),
            None,
        )
        .await;

    assert_resolves_successfully(response).await;
}

#[ntex::test]
async fn coprocessor_skip_enforcement_via_router_request() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    // Coprocessor sets skip_enforcement to true
    let router_request_mock = coprocessor
        .mock_stage("router.request")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "version": 1,
                "control": "continue",
                "context": {
                    "hive::persisted_documents::skip_enforcement": true
                }
            })
            .to_string(),
        )
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    router:
                      request:
                        include:
                          context: true
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    // Request has no ID
    let response = router
        .send_graphql_request("{ topProducts { name } }", None, None)
        .await;

    // Request should succeed, because we bypassed enforcement
    assert_resolves_successfully(response).await;
    router_request_mock.assert_async().await;
}

#[ntex::test]
async fn coprocessor_skip_enforcement_false_still_enforces() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    // Coprocessor sets skip_enforcement to false
    let router_request_mock = coprocessor
        .mock_stage("router.request")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "version": 1,
                "control": "continue",
                "context": {
                    "hive::persisted_documents::skip_enforcement": false
                }
            })
            .to_string(),
        )
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    router:
                      request:
                        include:
                          context: true
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    // Request has no ID
    let response = router
        .send_graphql_request("{ topProducts { name } }", None, None)
        .await;

    // Request should fail, because we did not bypass enforcement
    assert_error_code(response, "PERSISTED_DOCUMENT_ID_REQUIRED").await;
    router_request_mock.assert_async().await;
}

#[ntex::test]
async fn coprocessor_skip_enforcement_with_valid_document_id() {
    let manifest = write_manifest();
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let router_request_mock = coprocessor
        .mock_stage("router.request")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "version": 1,
                "control": "continue",
                "context": {
                    "hive::persisted_documents::skip_enforcement": true
                }
            })
            .to_string(),
        )
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(format!(
            r#"
                supergraph:
                  source: file
                  path: supergraph.graphql
                persisted_documents:
                  enabled: true
                  require_id: true
                  storage:
                    type: file
                    path: "{}"
                coprocessor:
                  url: http://{host}/coprocessor
                  protocol: http1
                  stages:
                    router:
                      request:
                        include:
                          headers: true
                "#,
            manifest.path().display(),
        ))
        .build()
        .start()
        .await;

    let response = router
        .send_post_request(
            "/graphql",
            json!({
                "documentId": "sha256:abc123"
            }),
            None,
        )
        .await;

    assert_resolves_successfully(response).await;
    router_request_mock.assert_async().await;
}
