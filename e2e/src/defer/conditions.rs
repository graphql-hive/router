//! `@defer(if:)` and `@skip`/`@include` around deferred fragments. With nothing deferred,
//! the response is a plain one, whatever the client accepts.

use insta::assert_snapshot;
use serde_json::json;

use super::*;

const IF_QUERY: &str = r#"query ($d: Boolean!) { me { id ... @defer(if: $d) { name reviews { id product { upc shippingEstimate } } } } }"#;
/// One plan covers every combination of `@defer(if:)` values: each fragment is planned both
/// ways, and the values pick which one runs.
#[ntex::test]
async fn one_plan_for_every_if_combination() {
    let setup = demo().await;
    let query = r#"query ($a: Boolean, $b: Boolean) { me { id ... @defer(if: $a) { name } ... @defer(if: $b) { username } } }"#;
    let mut runs = Vec::new();
    for variables in [
        json!({ "a": true, "b": true }),
        json!({ "a": true, "b": false }),
        json!({ "a": false, "b": true }),
        json!({ "a": false, "b": false }),
        // not sent is `true`, the default of `if`
        json!({ "b": false }),
    ] {
        let run = setup.send(query, Some(variables.clone())).await;
        // the responses, the subgraph log after them adds up over the runs
        let responses = run.text.lines().take_while(|line| line.starts_with('{'));
        runs.push(format!(
            "{variables}\n{}",
            responses.collect::<Vec<_>>().join("\n")
        ));
    }
    let runtime = setup
        .router
        .schema_state()
        .configured_runtime()
        .expect("configured runtime to exist");
    runtime.plan_cache.run_pending_tasks().await;
    assert_eq!(runtime.plan_cache.entry_count(), 1);
    assert_snapshot!(runs.join("\n"), @r#"
    {"a":true,"b":true}
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]},{"id":"1","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"username":"urigo"}}],"completed":[{"id":"1"}]}
    {"a":true,"b":false}
    {"data":{"me":{"id":"1","username":"urigo"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"0"}]}
    {"a":false,"b":true}
    {"data":{"me":{"id":"1","name":"Uri Goldshtein"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"username":"urigo"}}],"completed":[{"id":"0"}]}
    {"a":false,"b":false}
    {"data":{"me":{"id":"1","name":"Uri Goldshtein","username":"urigo"}}}
    {"b":false}
    {"data":{"me":{"id":"1","username":"urigo"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"0"}]}
    "#);
}

/// A conditional `@defer` in another one: the inner one is planned in both copies of the outer
/// one, so it's deferred from the primary part or from the outer part, whichever runs.
#[ntex::test]
async fn nested_if_defers_with_one_plan() {
    let setup = demo().await;
    let query = r#"query ($o: Boolean, $i: Boolean) { me { id ... @defer(if: $o) { name ... @defer(if: $i) { username } } } }"#;
    let mut runs = Vec::new();
    for variables in [
        json!({ "o": true, "i": true }),
        json!({ "o": true, "i": false }),
        json!({ "o": false, "i": true }),
        json!({ "o": false, "i": false }),
    ] {
        let run = setup.send(query, Some(variables.clone())).await;
        let responses = run.text.lines().take_while(|line| line.starts_with('{'));
        runs.push(format!(
            "{variables}\n{}",
            responses.collect::<Vec<_>>().join("\n")
        ));
    }
    let runtime = setup
        .router
        .schema_state()
        .configured_runtime()
        .expect("configured runtime to exist");
    runtime.plan_cache.run_pending_tasks().await;
    assert_eq!(runtime.plan_cache.entry_count(), 1);
    assert_snapshot!(runs.join("\n"), @r#"
    {"o":true,"i":true}
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"pending":[{"id":"1","path":["me"]}],"incremental":[{"id":"0","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"username":"urigo"}}],"completed":[{"id":"1"}]}
    {"o":true,"i":false}
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein","username":"urigo"}}],"completed":[{"id":"0"}]}
    {"o":false,"i":true}
    {"data":{"me":{"id":"1","name":"Uri Goldshtein"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"username":"urigo"}}],"completed":[{"id":"0"}]}
    {"o":false,"i":false}
    {"data":{"me":{"id":"1","name":"Uri Goldshtein","username":"urigo"}}}
    "#);
}

// Stripping `@defer(if: $d)` would leave `$d` unused and fail validation, so the plain
// query is written out.
const IF_PLAIN: &str = r#"{ me { id name reviews { id product { upc shippingEstimate } } } }"#;

#[ntex::test]
async fn if_true_defers() {
    let setup = demo().await;
    let d = setup
        .run(IF_QUERY, Some(json!({ "d": true })), Plain::Query(IF_PLAIN))
        .await;
    assert_snapshot!(d, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein","reviews":[{"id":"1","product":{"upc":"1","shippingEstimate":50}},{"id":"2","product":{"upc":"1","shippingEstimate":50}}]}}],"completed":[{"id":"0"}]}
    accounts: query($_d0:Boolean!){me{id ...on User @skip(if: $_d0){__typename name id} ...on User @include(if: $_d0){__typename id}}} {"_d0":true}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    inventory: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate}}} {"representations":[{"__typename":"Product","price":899,"weight":100,"upc":"1"}]}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price weight}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
/// `if: false` with a client that only takes JSON: a plain JSON response.
async fn if_false_gives_a_json_client_plain_json() {
    let setup = demo().await;
    assert_snapshot!(
        setup
            .raw(
                "query ($d: Boolean!) { me { id ... @defer(if: $d) { name } } }",
                Some(json!({ "d": false })),
                Some("application/json"),
            )
            .await,
        @r#"
    200 OK
    application/json
    {"data":{"me":{"id":"1","name":"Uri Goldshtein"}}}
    "#
    );
}

#[ntex::test]
/// An `@include`/`@skip` with a variable leaves the `@defer` out: on the fragment, on the
/// field around it, on a fragment around it. Nothing is deferred, so a client that only
/// takes JSON gets plain JSON, not a 406.
async fn excluded_defer_gives_a_json_client_plain_json() {
    let setup = demo().await;
    let mut out = Vec::new();
    for query in [
        "query ($show: Boolean!) { me { id ... @defer @include(if: $show) { name } } }",
        "query ($hide: Boolean!) { me { id ... @defer @skip(if: $hide) { name } } }",
        "query ($show: Boolean!) { me @include(if: $show) { id ... @defer { name } } }",
        "query ($show: Boolean!) { me { id ... @include(if: $show) { ... @defer { name } } } }",
    ] {
        out.push(
            setup
                .raw(
                    query,
                    Some(if query.contains("$hide") {
                        json!({ "hide": true })
                    } else {
                        json!({ "show": false })
                    }),
                    Some("application/json"),
                )
                .await,
        );
    }
    assert_snapshot!(out.join("\n"), @r#"
    200 OK
    application/json
    {"data":{"me":{"id":"1"}}}
    200 OK
    application/json
    {"data":{"me":{"id":"1"}}}
    200 OK
    application/json
    {"data":{}}
    200 OK
    application/json
    {"data":{"me":{"id":"1"}}}
    "#);
}

#[ntex::test]
/// `@defer(if: $shouldDefer)` with a nullable variable.
/// Deferred with `true` and when the variable isn't sent (the argument's default kicks in),
/// plain with `false`.
async fn missing_if_variable_defers() {
    let setup = demo().await;
    let query = r#"
        query Me($shouldDefer: Boolean) {
            me {
              id
              ... @defer(if: $shouldDefer) {
                name
                username
              }
            }
          }"#;
    let on = setup
        .check(query, Some(json!({ "shouldDefer": true })))
        .await;
    assert_snapshot!(on.text, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein","username":"urigo"}}],"completed":[{"id":"0"}]}
    accounts: query($_d0:Boolean!){me{id ...on User @skip(if: $_d0){name username} ...on User @include(if: $_d0){__typename id}}} {"_d0":true}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name username}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
    // `if` defaults to `true`, so a variable that isn't sent means deferred
    let missing = setup.send(query, None).await;
    assert_snapshot!(missing.text, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein","username":"urigo"}}],"completed":[{"id":"0"}]}
    accounts: query($_d0:Boolean!){me{id ...on User @skip(if: $_d0){name username} ...on User @include(if: $_d0){__typename id}}} {"_d0":false}
    accounts: query($_d0:Boolean!){me{id ...on User @skip(if: $_d0){name username} ...on User @include(if: $_d0){__typename id}}} {"_d0":true}
    accounts: query($_d0:Boolean!){me{id ...on User @skip(if: $_d0){name username} ...on User @include(if: $_d0){__typename id}}} {"_d0":true}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name username}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name username}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
    let (content_type, body) = setup
        .send_raw(
            query,
            Some(json!({ "shouldDefer": false })),
            "multipart/mixed",
        )
        .await;
    assert_snapshot!(format!("{content_type}\n{body}"), @r#"
    multipart/mixed;boundary="-"
    ---
    Content-Type: application/json

    {"data":{"me":{"id":"1","name":"Uri Goldshtein","username":"urigo"}}}
    -----
    "#);
    assert_eq!(
        missing.parts.len(),
        2,
        "`@defer(if: $shouldDefer)` without the variable should defer"
    );
}

#[ntex::test]
/// `if: $if` with `$if: Boolean! = true`.
/// Deferred without variables, one plain response (no `hasNext`) with `if: false`.
async fn if_variable_default_applies() {
    let setup = demo().await;
    let query = r#"query X($if: Boolean! = true) {
        me {
            id
            ...@defer(label: "name", if: $if) {
                name
            }
        }
    }"#;
    let deferred = setup.check(query, None).await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"],"label":"name"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"0"}]}
    accounts: query($_d0:Boolean!){me{id ...on User @skip(if: $_d0){name} ...on User @include(if: $_d0){__typename id}}} {"_d0":true}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
    // `if: false` gives a plain response, so a client that only takes multipart should still
    // get one it can read
    let (content_type, body) = setup
        .send_raw(query, Some(json!({ "if": false })), "multipart/mixed")
        .await;
    assert_snapshot!(format!("{content_type}\n{body}"), @r#"
    multipart/mixed;boundary="-"
    ---
    Content-Type: application/json

    {"data":{"me":{"id":"1","name":"Uri Goldshtein"}}}
    -----
    "#);
    let (content_type, body) = setup
        .send_raw(query, Some(json!({ "if": false })), LEGACY)
        .await;
    assert_snapshot!(format!("{content_type}\n{body}"), @r#"
    multipart/mixed;boundary="-";deferSpec=20220824
    ---
    Content-Type: application/json

    {"data":{"me":{"id":"1","name":"Uri Goldshtein"}}}
    -----
    "#);
}

/// `@include(if: false)` on the deferred fragment: one plain payload with `{user:{name}}`,
/// no reviews, no `pending`/`incremental`.
#[ntex::test]
async fn include_false_drops_the_deferred_fragment() {
    let setup = start(user_reviews_mocks()).await;
    let parts = setup
        .run_accepting_json(
            r#"
              query GetUser {
                user(id: "1") {
                  name
                  ... @defer(label: "reviews") @include(if: false) {
                    reviews { title body }
                  }
                }
              }
            "#,
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    content-type: application/graphql-response+json
    {"data":{"user":{"name":"Ada"}}}
    accounts: query($_v0:ID!){user(id: $_v0){name}} {"_v0":"1"}
    "#);
}
