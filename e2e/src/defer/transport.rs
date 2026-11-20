//! How the parts go out: `multipart/mixed` in the v0.2 or the legacy format, SSE, WebSocket,
//! which `Accept` gets which, and when `@defer` is refused.

use insta::assert_snapshot;
use serde_json::Value;

use super::*;

const ACCEPT_QUERY: &str = r#"query { me { id ... @defer { name } } }"#;

#[ntex::test]
async fn accept_missing() {
    assert_snapshot!(demo().await.raw(ACCEPT_QUERY, None, None).await, @r#"
    200 OK
    multipart/mixed;boundary="-";incrementalSpec=v0.2
    ---
    Content-Type: application/json

    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    ---
    Content-Type: application/json

    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"0"}],"hasNext":false}
    -----
    "#);
}

#[ntex::test]
async fn accept_json_only() {
    assert_snapshot!(demo().await.raw(ACCEPT_QUERY, None, Some("application/json")).await, @r#"
    406 Not Acceptable
    application/json
    {"errors":[{"message":"The operation uses @defer, but the client doesn't accept an incremental response. Accept multipart/mixed or text/event-stream.","extensions":{"code":"DEFER_TRANSPORT_NOT_SUPPORTED"}}]}
    "#);
}

#[ntex::test]
/// `normal_query_with_defer_accept_header`: no `@defer`, but the client accepts multipart.
/// One plain response, no `hasNext`.
async fn multipart_client_gets_a_response_without_defer() {
    let setup = demo().await;
    let (content_type, body) = setup
        .send_raw(
            "{ me { reviews { author { reviews { author { name } } } } } }",
            None,
            LEGACY,
        )
        .await;
    assert_snapshot!(format!("{content_type}\n{body}"), @r#"
    multipart/mixed;boundary="-";deferSpec=20220824
    ---
    Content-Type: application/json

    {"data":{"me":{"reviews":[{"author":{"reviews":[{"author":{"name":"Uri Goldshtein"}},{"author":{"name":"Uri Goldshtein"}}]}},{"author":{"reviews":[{"author":{"name":"Uri Goldshtein"}},{"author":{"name":"Uri Goldshtein"}}]}}]}}}
    -----
    "#);
    assert_has_data(&body);
}

#[ntex::test]
/// The Apollo multipart protocol is for subscriptions, a client that only takes it gets no
/// query response, its parts would be in a framing it can't read.
async fn apollo_multipart_only_client_gets_no_query_response() {
    let setup = demo().await;
    let response = setup
        .router
        .serv()
        .post(setup.router.graphql_path())
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(
            http::header::ACCEPT,
            r#"multipart/mixed;subscriptionSpec="1.0""#,
        )
        .send_json(&serde_json::json!({ "query": "{ me { id } }" }))
        .await
        .unwrap();
    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = response.string_body().await;
    // the protocol's own error framing, not incremental delivery
    assert!(content_type.contains("subscriptionSpec"), "{content_type}");
    assert!(body.contains("UNSUPPORTED_CONTENT_TYPE"), "{body}");
}

// ---- the legacy format and SSE ----

const LIST_OF_OBJECTS: &str = r#"{ topProducts(first: 1) { upc name reviews { id ... @defer { author { id name username } } } } }"#;
// nested, so the parts come in a fixed order
const LABELED: &str = r#"{ topProducts(first: 2) { upc name ... @defer(label: "stock") { inStock ... @defer(label: "shipping") { shippingEstimate } } } }"#;

#[ntex::test]
async fn legacy_paths_in_a_list() {
    let setup = demo().await;
    let (content_type, body) = setup
        .send_raw(LIST_OF_OBJECTS, None, "multipart/mixed;deferSpec=20220824")
        .await;
    assert_eq!(
        content_type,
        r#"multipart/mixed;boundary="-";deferSpec=20220824"#
    );
    assert_snapshot!(body, @r#"
    ---
    Content-Type: application/json

    {"data":{"topProducts":[{"upc":"1","name":"Table","reviews":[{"id":"1"},{"id":"2"},{"id":"3"},{"id":"4"}]}]},"hasNext":true}
    ---
    Content-Type: application/json

    {"incremental":[{"data":{"author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}},"path":["topProducts",0,"reviews",0]},{"data":{"author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}},"path":["topProducts",0,"reviews",1]},{"data":{"author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}},"path":["topProducts",0,"reviews",2]},{"data":{"author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}},"path":["topProducts",0,"reviews",3]}],"hasNext":false}
    -----
    "#);
}

#[ntex::test]
async fn legacy_labels() {
    let setup = demo().await;
    let (_, body) = setup
        .send_raw(LABELED, None, "multipart/mixed;deferSpec=20220824")
        .await;
    assert_snapshot!(body, @r#"
    ---
    Content-Type: application/json

    {"data":{"topProducts":[{"upc":"1","name":"Table"},{"upc":"2","name":"Couch"}]},"hasNext":true}
    ---
    Content-Type: application/json

    {"incremental":[{"data":{"inStock":true},"path":["topProducts",0],"label":"stock"},{"data":{"inStock":false},"path":["topProducts",1],"label":"stock"}],"hasNext":true}
    ---
    Content-Type: application/json

    {"incremental":[{"data":{"shippingEstimate":50},"path":["topProducts",0],"label":"shipping"},{"data":{"shippingEstimate":0},"path":["topProducts",1],"label":"shipping"}],"hasNext":false}
    -----
    "#);
}

#[ntex::test]
async fn sse_parts_merge_like_multipart() {
    let setup = demo().await;
    let (content_type, body) = setup
        .send_raw(LIST_OF_OBJECTS, None, "text/event-stream")
        .await;
    assert_eq!(content_type, "text/event-stream");
    // same parts as multipart, so merging them must give the plain response too
    let parts: Vec<Value> = body
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let plain = send_plain(&setup.router, LIST_OF_OBJECTS, None).await;
    assert_eq!(merge_parts(&parts), plain);
    assert_snapshot!(body, @r#"
    event: next
    data: {"data":{"topProducts":[{"upc":"1","name":"Table","reviews":[{"id":"1"},{"id":"2"},{"id":"3"},{"id":"4"}]}]},"pending":[{"id":"0","path":["topProducts",0,"reviews",0]},{"id":"1","path":["topProducts",0,"reviews",1]},{"id":"2","path":["topProducts",0,"reviews",2]},{"id":"3","path":["topProducts",0,"reviews",3]}],"hasNext":true}

    event: next
    data: {"incremental":[{"id":"0","data":{"author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}}},{"id":"1","data":{"author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}}},{"id":"2","data":{"author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}}},{"id":"3","data":{"author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"},{"id":"3"}],"hasNext":false}

    event: complete
    "#);
}

#[ntex::test]
async fn sse_labels() {
    let setup = demo().await;
    let (_, body) = setup.send_raw(LABELED, None, "text/event-stream").await;
    assert_snapshot!(body, @r#"
    event: next
    data: {"data":{"topProducts":[{"upc":"1","name":"Table"},{"upc":"2","name":"Couch"}]},"pending":[{"id":"0","path":["topProducts",0],"label":"stock"},{"id":"1","path":["topProducts",1],"label":"stock"}],"hasNext":true}

    event: next
    data: {"pending":[{"id":"2","path":["topProducts",0],"label":"shipping"},{"id":"3","path":["topProducts",1],"label":"shipping"}],"incremental":[{"id":"0","data":{"inStock":true}},{"id":"1","data":{"inStock":false}}],"completed":[{"id":"0"},{"id":"1"}],"hasNext":true}

    event: next
    data: {"incremental":[{"id":"2","data":{"shippingEstimate":50}},{"id":"3","data":{"shippingEstimate":0}}],"completed":[{"id":"2"},{"id":"3"}],"hasNext":false}

    event: complete
    "#);
}

// ---- refused ----

const LABELED_QUERY: &str = r#"
    query {
        me {
            id
            ... @defer(label: "name") {
                name
            }
        }
    }
"#;

/// `limits.max_defers` counts every `@defer`, `@defer(if:)` included.
#[ntex::test]
async fn too_many_defers() {
    let setup = start_full(
        "supergraph.graphql",
        None,
        "limits:\n  max_defers:\n    n: 2\n",
    )
    .await;
    let two = "{ me { id ... @defer { name } ... @defer { username } } }";
    assert!(!setup
        .raw(two, None, Some("multipart/mixed"))
        .await
        .contains("MAX_DEFERS_EXCEEDED"));
    let three = "query ($d: Boolean) { me { id ... @defer { name } ... @defer { username } ... @defer(if: $d) { name } } }";
    assert_snapshot!(setup.raw(three, None, Some("multipart/mixed")).await, @r#"
    200 OK
    multipart/mixed;boundary="-"
    ---
    Content-Type: application/json

    {"errors":[{"message":"Defer limit exceeded.","locations":[{"line":1,"column":79}],"extensions":{"code":"MAX_DEFERS_EXCEEDED"}}]}
    -----
    "#);
}

/// `limits.max_defers` is 4 by default: a fifth `@defer` is too many.
#[ntex::test]
async fn too_many_defers_by_default() {
    let setup = demo().await;
    let query = |n: usize| {
        let vars: Vec<_> = (0..n).map(|i| format!("$v{i}: Boolean")).collect();
        let defers: Vec<_> = (0..n)
            .map(|i| format!("... @defer(if: $v{i}) {{ name }}"))
            .collect();
        format!(
            "query ({}) {{ me {{ id {} }} }}",
            vars.join(", "),
            defers.join(" ")
        )
    };
    assert!(!setup
        .raw(&query(4), None, Some("multipart/mixed"))
        .await
        .contains("MAX_DEFERS_EXCEEDED"));
    let too_many = setup.raw(&query(5), None, Some("multipart/mixed")).await;
    assert!(too_many.contains(r#""message":"Defer limit exceeded.""#));
    assert!(too_many.contains("MAX_DEFERS_EXCEEDED"));
}

#[ntex::test]
async fn rejected_when_disabled() {
    let setup = start_full("supergraph.graphql", None, "defer:\n  enabled: false\n").await;
    assert_snapshot!(setup.raw(LABELED_QUERY, None, Some("multipart/mixed")).await, @r#"
    200 OK
    multipart/mixed;boundary="-"
    ---
    Content-Type: application/json

    {"errors":[{"message":"Unknown directive \"@defer\".","locations":[{"line":5,"column":17}],"extensions":{"code":"KnownDirectives"}}]}
    -----
    "#);
}

#[ntex::test]
async fn rejected_in_subscriptions() {
    let setup = start_full(
        "supergraph.graphql",
        None,
        "subscriptions:\n  enabled: true\n",
    )
    .await;
    let sse = Some("text/event-stream");
    assert_snapshot!(
        setup
            .raw(
                "subscription { reviewAdded(intervalInMs: 0) { product { ... @defer { upc } } } }",
                None,
                sse,
            )
            .await,
        @r#"
    200 OK
    text/event-stream
    event: next
    data: {"errors":[{"message":"`@defer` is not supported in subscriptions.","locations":[{"line":1,"column":61}],"extensions":{"code":"DEFER_IN_SUBSCRIPTION"}}]}

    event: complete
    "#
    );
    // turned off, it's still rejected
    assert_snapshot!(
        setup
            .raw(
                "subscription { reviewAdded(intervalInMs: 0) { product { ... @defer(if: false) { upc } } } }",
                None,
                sse,
            )
            .await,
        @r#"
    200 OK
    text/event-stream
    event: next
    data: {"errors":[{"message":"`@defer` is not supported in subscriptions.","locations":[{"line":1,"column":61}],"extensions":{"code":"DEFER_IN_SUBSCRIPTION"}}]}

    event: complete
    "#
    );
}

// ---- headers and timeouts, which go out with the primary part ----

/// The deferred `inStock` comes after the primary part, from a subgraph that says `no-store`.
/// The policy goes out before that answer, so a deferred response is never cached: when the
/// primary subgraph says `public`, and when it says nothing.
#[ntex::test]
async fn deferred_response_is_never_cached() {
    for primary in [Some("public, max-age=300"), None] {
        let subgraphs = TestSubgraphs::builder()
            .with_on_request(move |req| {
                let cache_control = match req.path.as_str() {
                    "/products" => primary?,
                    "/inventory" => "no-store",
                    _ => return None,
                };
                Some(ResponseLike::new(
                    axum::http::StatusCode::OK,
                    None,
                    some_header_map! { http::header::CACHE_CONTROL => cache_control },
                ))
            })
            .with_path_delay("/inventory", std::time::Duration::from_millis(300))
            .build()
            .start()
            .await;
        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .inline_config(
                "supergraph:\n  source: file\n  path: supergraph.graphql\ndefer:\n  enabled: true\nheaders:\n  all:\n    response:\n      - propagate:\n          named: cache-control\n          algorithm: append\n",
            )
            .build()
            .start()
            .await;
        let response = router
            .send_graphql_request(
                "{ topProducts(first: 1) { upc ... @defer { inStock } } }",
                None,
                some_header_map! { http::header::ACCEPT => "multipart/mixed" },
            )
            .await;
        let cache_control = response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.string_body().await;
        assert!(body.contains("inStock"), "{body}");
        assert_eq!(
            cache_control, "no-store, no-cache, must-revalidate",
            "primary subgraph sent {primary:?}"
        );
    }
}

/// The router's request timeout covers the deferred parts too: the slow `inStock` call is cut
/// off, and the last part ends the pending fragment with the timeout error, in either format.
#[ntex::test]
async fn request_timeout_ends_the_deferred_parts() {
    let subgraphs = TestSubgraphs::builder()
        .with_path_delay("/inventory", std::time::Duration::from_millis(3000))
        .build()
        .start()
        .await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(
            "supergraph:\n  source: file\n  path: supergraph.graphql\ndefer:\n  enabled: true\ntraffic_shaping:\n  router:\n    request_timeout: 1s\n",
        )
        .build()
        .start()
        .await;
    let setup = Setup { subgraphs, router };
    let mut out = Vec::new();
    for accept in ["multipart/mixed", LEGACY] {
        let started = std::time::Instant::now();
        let (_, body) = setup
            .send_raw(
                "{ topProducts(first: 1) { upc ... @defer { inStock } } }",
                None,
                accept,
            )
            .await;
        assert!(
            started.elapsed() < std::time::Duration::from_millis(2500),
            "the response waited for the slow call: {:?}",
            started.elapsed()
        );
        out.extend(multipart_parts(&body).iter().map(Value::to_string));
    }
    assert_snapshot!(out.join("\n"), @r#"
    {"data":{"topProducts":[{"upc":"1"}]},"pending":[{"id":"0","path":["topProducts",0]}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Request timed out","extensions":{"code":"GATEWAY_TIMEOUT"}}]}],"hasNext":false}
    {"data":{"topProducts":[{"upc":"1"}]},"hasNext":true}
    {"incremental":[{"data":null,"path":["topProducts",0],"errors":[{"message":"Request timed out","extensions":{"code":"GATEWAY_TIMEOUT"}}]}],"hasNext":false}
    "#);
}

/// With nothing deferred, the one part keeps the `@defer` version the client asked for, the
/// same content type `@defer(if: true)` gets.
#[ntex::test]
async fn nothing_deferred_keeps_the_negotiated_version() {
    let setup = demo().await;
    let query = "query ($d: Boolean!) { me { id ... @defer(if: $d) { name } } }";
    let mut out = Vec::new();
    for accept in [LEGACY, "multipart/mixed;incrementalSpec=v0.2"] {
        for d in [true, false] {
            let (content_type, _) = setup
                .send_raw(query, Some(serde_json::json!({ "d": d })), accept)
                .await;
            out.push(format!("{accept} if: {d} -> {content_type}"));
        }
    }
    assert_snapshot!(out.join("\n"), @r#"
    multipart/mixed;deferSpec=20220824 if: true -> multipart/mixed;boundary="-";deferSpec=20220824
    multipart/mixed;deferSpec=20220824 if: false -> multipart/mixed;boundary="-";deferSpec=20220824
    multipart/mixed;incrementalSpec=v0.2 if: true -> multipart/mixed;boundary="-";incrementalSpec=v0.2
    multipart/mixed;incrementalSpec=v0.2 if: false -> multipart/mixed;boundary="-";incrementalSpec=v0.2
    "#);
}

// ---- WebSocket ----

#[ntex::test]
async fn websocket_sends_one_message_per_part() {
    use futures::StreamExt;
    use hive_router::executor::executors::{
        graphql_transport_ws::SubscribePayload, websocket_client::WsClient,
    };

    let setup = start_full("supergraph.graphql", None, "websocket:\n  enabled: true\n").await;
    let wsconn = setup.router.ws().await;
    let mut client = WsClient::new(wsconn)
        .init(None)
        .await
        .expect("Failed to init WsClient");

    let mut stream = client
        .subscribe(
            SubscribePayload {
                query: LABELED_QUERY.into(),
                ..Default::default()
            },
            None,
        )
        .await
        .expect("Failed to subscribe");

    let primary = stream
        .next()
        .await
        .expect("primary part")
        .expect("primary part parses");
    assert_snapshot!(sonic_rs::to_string(&primary.data).unwrap(), @r#"{"me":{"id":"1"}}"#);
    // the test client only parses `data`/`errors` responses, so the incremental
    // part comes back as a parse error, it's still one `next` message
    assert!(stream.next().await.is_some(), "expected the deferred part");
    assert!(stream.next().await.is_none(), "expected the stream to end");
}

#[ntex::test]
/// A client can cancel a deferred operation with `complete`: the deferred fetches that haven't
/// started never do.
async fn websocket_deferred_operation_can_be_cancelled() {
    use crate::testkit::{TestRouter, TestSubgraphs};
    use futures::StreamExt;
    use hive_router::executor::executors::{
        graphql_transport_ws::SubscribePayload, websocket_client::WsClient,
    };
    use std::time::Duration;

    let subgraphs = TestSubgraphs::builder()
        .with_path_delay("/reviews", Duration::from_millis(1000))
        .build()
        .start()
        .await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(
            "supergraph:\n  source: file\n  path: supergraph.graphql\ndefer:\n  enabled: true\nwebsocket:\n  enabled: true\n",
        )
        .build()
        .start()
        .await;
    let mut client = WsClient::new(router.ws().await)
        .init(None)
        .await
        .expect("Failed to init WsClient");

    // the deferred part reads the slow reviews subgraph, then accounts for the authors
    let mut stream = client
        .subscribe(
            SubscribePayload {
                query:
                    "{ topProducts(first: 1) { upc ... @defer { reviews { author { name } } } } }"
                        .into(),
                ..Default::default()
            },
            None,
        )
        .await
        .expect("Failed to subscribe");
    let primary = stream
        .next()
        .await
        .expect("primary part")
        .expect("primary part parses");
    assert_snapshot!(sonic_rs::to_string(&primary.data).unwrap(), @r#"{"topProducts":[{"upc":"1"}]}"#);
    // sends `complete` while the reviews call is still running
    drop(stream);

    ntex::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        subgraphs
            .get_requests_log("accounts")
            .unwrap_or_default()
            .is_empty(),
        "the cancelled operation still called accounts"
    );
}
