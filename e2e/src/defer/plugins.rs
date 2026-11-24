//! Plugin hooks with deferred operations: the `on_execute` end hook runs once, before the
//! primary part, and the deferred fetches go through the subgraph hook too.

use std::sync::atomic::{AtomicUsize, Ordering};

use hive_router::plugins::hooks::on_execute::{
    OnExecuteEndHookPayload, OnExecuteStartHookPayload, OnExecuteStartHookResult,
};
use hive_router::plugins::hooks::on_graphql_error::{
    OnGraphQLErrorHookPayload, OnGraphQLErrorHookResult,
};
use hive_router::plugins::hooks::on_plugin_init::{OnPluginInitPayload, OnPluginInitResult};
use hive_router::plugins::hooks::on_subgraph_execute::{
    OnSubgraphExecuteStartHookPayload, OnSubgraphExecuteStartHookResult,
};
use hive_router::plugins::plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload};

use super::Setup;
use crate::testkit::{TestRouter, TestSubgraphs};

#[derive(Default)]
struct DeferHooksPlugin;

static SUBGRAPH_CALLS: AtomicUsize = AtomicUsize::new(0);

#[hive_router::async_trait]
impl RouterPlugin for DeferHooksPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "test_defer_hooks"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }

    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        payload.on_end(|mut end_payload: OnExecuteEndHookPayload<'exec>| {
            let has_next = end_payload.has_next;
            end_payload.add_extension("has_next", has_next);
            end_payload.proceed()
        })
    }

    async fn on_subgraph_execute<'exec>(
        &'exec self,
        payload: OnSubgraphExecuteStartHookPayload<'exec>,
    ) -> OnSubgraphExecuteStartHookResult<'exec> {
        SUBGRAPH_CALLS.fetch_add(1, Ordering::Relaxed);
        payload.proceed()
    }
}

#[ntex::test]
async fn hooks_run_for_deferred_operations() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            defer:
                enabled: true
            plugins:
                test_defer_hooks:
                    enabled: true
            "#,
        )
        .register_plugin::<DeferHooksPlugin>()
        .build()
        .start()
        .await;
    let setup = Setup { subgraphs, router };
    let (_, body) = setup
        .send_raw(
            "{ topProducts(first: 1) { upc ... @defer { reviews { body } } } }",
            None,
            "multipart/mixed",
        )
        .await;
    // the end hook ran once, before the primary part
    assert!(body.contains(r#""extensions":{"has_next":true}"#), "{body}");
    // the deferred `reviews` fetch went through the subgraph hook too
    assert_eq!(SUBGRAPH_CALLS.load(Ordering::Relaxed), 2, "{body}");
}

/// Answers any GraphQL error with a 500
#[derive(Default)]
struct ErrorStatusPlugin;

#[hive_router::async_trait]
impl RouterPlugin for ErrorStatusPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "test_error_status"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }

    fn on_graphql_error<'req>(
        &self,
        mut payload: OnGraphQLErrorHookPayload<'req>,
    ) -> OnGraphQLErrorHookResult<'req> {
        payload.status_code = http::StatusCode::INTERNAL_SERVER_ERROR;
        payload.proceed()
    }
}

/// `inventory` fails, and the plugin turns any error into a 500
async fn failing_inventory_with_error_status() -> Setup {
    let subgraphs = TestSubgraphs::builder()
        .with_on_request(|req| {
            req.path.contains("inventory").then(|| {
                crate::testkit::ResponseLike::new(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    Some(r#"{"errors":[{"message":"inventory is down"}]}"#.to_string()),
                    crate::some_header_map! { http::header::CONTENT_TYPE => "application/json" },
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
            defer:
                enabled: true
            plugins:
                test_error_status:
                    enabled: true
            "#,
        )
        .register_plugin::<ErrorStatusPlugin>()
        .build()
        .start()
        .await;
    Setup { subgraphs, router }
}

/// A multipart response keeps the status a regular one gets: a regular response to a client
/// that only takes `multipart/mixed`, and a deferred one with errors in its primary part.
#[ntex::test]
async fn multipart_responses_keep_the_status() {
    let setup = failing_inventory_with_error_status().await;
    let accept = Some("multipart/mixed");

    // nothing deferred, sent as the only part
    let plain = setup
        .raw("{ topProducts(first: 1) { upc inStock } }", None, accept)
        .await;
    assert!(plain.starts_with("500 "), "{plain}");
    assert!(plain.contains("inventory"), "{plain}");

    // the error is in the primary part, the head goes out with it
    let primary = setup
        .raw(
            "{ topProducts(first: 1) { upc inStock ... @defer { name } } }",
            None,
            accept,
        )
        .await;
    assert!(primary.starts_with("500 "), "{primary}");
    assert!(primary.contains(r#""hasNext":true"#), "{primary}");

    // the error is only in a deferred part, the head is out by then
    let deferred = setup
        .raw(
            "{ topProducts(first: 1) { upc ... @defer { inStock } } }",
            None,
            accept,
        )
        .await;
    assert!(deferred.starts_with("200 "), "{deferred}");
    assert!(deferred.contains("inventory"), "{deferred}");
}

/// An error on a deferred field can come in before the primary part is out, when the deferred
/// fetch is the faster one. It still goes out with its part, and doesn't decide the primary
/// part's status.
#[ntex::test]
async fn an_early_deferred_error_leaves_the_status() {
    let subgraphs = TestSubgraphs::builder()
        .with_on_request(|req| {
            req.path.contains("accounts").then(|| {
                crate::testkit::ResponseLike::new(
                    http::StatusCode::OK,
                    Some(
                        r#"{"data":{"me":null},"errors":[{"message":"no user","path":["me"]}]}"#
                            .to_string(),
                    ),
                    crate::some_header_map! { http::header::CONTENT_TYPE => "application/json" },
                )
            })
        })
        // the primary part waits for `products`, the deferred `me` is in long before
        .with_path_delay("/products", std::time::Duration::from_millis(500))
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
            defer:
                enabled: true
            plugins:
                test_error_status:
                    enabled: true
            "#,
        )
        .register_plugin::<ErrorStatusPlugin>()
        .build()
        .start()
        .await;
    let setup = Setup { subgraphs, router };
    let response = setup
        .raw(
            "{ topProducts(first: 1) { upc } ... @defer { me { id } } }",
            None,
            Some("multipart/mixed"),
        )
        .await;
    assert!(response.starts_with("200 "), "{response}");
    // the error went out with the deferred part, not the primary one
    let primary = response
        .lines()
        .find(|line| line.contains(r#""hasNext":true"#))
        .unwrap_or_default();
    assert!(!primary.contains(r#""service":"accounts""#), "{response}");
    assert!(response.contains(r#""service":"accounts""#), "{response}");
}

/// Replaces whatever errors the end hook gets with its own one, and says how many it got.
#[derive(Default)]
struct ReplaceErrorsPlugin;

#[hive_router::async_trait]
impl RouterPlugin for ReplaceErrorsPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "test_replace_errors"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }

    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        payload.on_end(|mut end_payload: OnExecuteEndHookPayload<'exec>| {
            let seen = end_payload.errors.len();
            end_payload.add_extension("errors_seen", seen);
            end_payload.errors.clear();
            end_payload.with_error(hive_router::GraphQLError::from_message_and_code(
                "hook error",
                "HOOK_ERROR",
            ));
            end_payload.proceed()
        })
    }
}

/// `products` is down and answers before the slow primary `accounts` call. Its error is the
/// deferred part's, so the end hook, which runs for the primary part, never sees it, and the
/// error the hook puts in its place stays in the primary part.
#[ntex::test]
async fn end_hooks_only_get_the_primary_part_errors() {
    let subgraphs = TestSubgraphs::builder()
        .with_on_request(|req| {
            (req.path == "/products").then(|| {
                crate::testkit::ResponseLike::new(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    Some("boom".to_string()),
                    None,
                )
            })
        })
        .with_path_delay("/accounts", std::time::Duration::from_millis(500))
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
            defer:
                enabled: true
            plugins:
                test_replace_errors:
                    enabled: true
            "#,
        )
        .register_plugin::<ReplaceErrorsPlugin>()
        .build()
        .start()
        .await;
    let setup = Setup { subgraphs, router };
    let (_, body) = setup
        .send_raw(
            "{ me { id } ... @defer { topProducts(first: 1) { upc } } }",
            None,
            "multipart/mixed",
        )
        .await;
    let parts = super::multipart_parts(&body);
    assert_eq!(parts.len(), 2, "{body}");
    assert_eq!(parts[0]["extensions"]["errors_seen"], 0, "{body}");
    assert_eq!(parts[0]["errors"][0]["message"], "hook error", "{body}");
    assert_eq!(
        parts[1]["incremental"][0]["errors"][0]["extensions"]["service"], "products",
        "{body}"
    );
}

/// Answers every operation with a plain 403 from `on_execute`.
#[derive(Default)]
struct ForbidPlugin;

#[hive_router::async_trait]
impl RouterPlugin for ForbidPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "test_forbid"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }

    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        payload.end_with_response(hive_router::plugins::plugin_trait::EarlyHTTPResponse {
            body: br#"{"message":"forbidden"}"#.to_vec(),
            headers: http::HeaderMap::new(),
            status_code: http::StatusCode::FORBIDDEN,
        })
    }
}

/// A plugin's own response to a deferred operation goes out as it is, like it would without
/// `@defer`, not as a part of an incremental response.
#[ntex::test]
async fn a_plugin_response_is_not_a_part() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(
            r#"
            supergraph:
                source: file
                path: supergraph.graphql
            defer:
                enabled: true
            plugins:
                test_forbid:
                    enabled: true
            "#,
        )
        .register_plugin::<ForbidPlugin>()
        .build()
        .start()
        .await;
    let setup = Setup { subgraphs, router };
    let response = setup
        .raw(
            "{ topProducts(first: 1) { upc ... @defer { name } } }",
            None,
            Some("multipart/mixed;incrementalSpec=v0.2, application/json"),
        )
        .await;
    assert_eq!(
        response,
        "403 Forbidden\napplication/json\n{\"message\":\"forbidden\"}"
    );
}
