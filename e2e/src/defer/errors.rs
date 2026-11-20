//! Subgraph errors and nulls in deferred fragments: an error goes out with the part that owns
//! it, a null stops at the fragment, and a fragment under a nulled parent is never sent.

use insta::assert_snapshot;
use serde_json::{json, Value};

use super::*;

#[ntex::test]
/// The `name` error comes from the deferred `name` call and goes out with its patch.
async fn deferred_field_error_keeps_its_patch() {
    let setup = start_full(
        ORGA,
        canned(vec![
            (
                "user",
                "_entities",
                json!({
                    "data": { "_entities": [{ "__typename": "User", "id": "0", "name": null }] },
                    "errors": [{ "message": "error user 0", "path": ["_entities", 0, "name"] }]
                }),
            ),
            // the same query without `@defer`
            (
                "user",
                "currentUser{id name}",
                json!({
                    "data": { "currentUser": { "__typename": "User", "id": "0", "name": null } },
                    "errors": [{ "message": "error user 0", "path": ["currentUser", "name"] }]
                }),
            ),
            (
                "user",
                "currentUser",
                json!({ "data": { "currentUser": { "__typename": "User", "id": "0" } } }),
            ),
        ]),
        "",
    )
    .await;
    let deferred = setup
        .check("query { currentUser { id ...@defer { name } } }", None)
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"currentUser":{"id":"0"}},"pending":[{"id":"0","path":["currentUser"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":null},"errors":[{"message":"Unexpected error","path":["currentUser","name"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"user","affectedPath":"currentUser"}}]}],"completed":[{"id":"0"}]}
    user: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"0"}]}
    user: {currentUser{__typename id}}
    "#);
}

#[ntex::test]
/// `errors_from_primary_on_deferred_responses`: the deferred field's error comes with the
/// primary fetch, but belongs in the deferred part.
async fn primary_fetch_error_goes_to_the_deferred_part() {
    let setup = start_full(
        COMPUTERS,
        canned(vec![(
            "computers",
            "computer",
            json!({
                "data": { "computer": { "id": "Computer1" } },
                "errors": [{
                    "message": "Error field",
                    "locations": [{ "line": 1, "column": 93 }],
                    "path": ["computer", "errorField"]
                }]
            }),
        )]),
        "",
    )
    .await;
    let deferred = setup
        .check(
            r#"query {
            computer(id: "Computer1") {
              id
              ...ComputerErrorField @defer
            }
          }
          fragment ComputerErrorField on Computer {
            errorField
          }"#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"computer":{"id":"Computer1"}},"pending":[{"id":"0","path":["computer"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"errorField":null},"errors":[{"message":"Unexpected error","locations":[{"line":1,"column":93}],"path":["computer","errorField"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"computers"}}]}],"completed":[{"id":"0"}]}
    computers: query($_v0:ID!){computer(id: $_v0){id errorField}} {"_v0":"Computer1"}
    "#);
}

#[ntex::test]
/// `errors_with_invalid_paths_on_query_with_defer`: subgraph error paths that aren't in the
/// response are dropped or cut back to the part that is.
async fn invalid_error_paths_are_dropped_or_cut_back() {
    let setup = start_full(
        COMPUTERS,
        canned(vec![(
            "computers",
            "computer",
            json!({
                "data": { "computer": { "id": "Computer1" } },
                "errors": [
                    { "message": "Subgraph error with invalid path", "path": ["invalid", "path"] },
                    { "message": "Subgraph error with partially valid path", "path": ["currentUser", "invalidSubpath"] },
                    { "message": "Error field", "path": ["computer", "errorField", "errorFieldSubpath"] }
                ]
            }),
        )]),
        "",
    )
    .await;
    let deferred = setup
        .check(
            r#"query {
            computer(id: "Computer1") {
              id
              ...ComputerErrorField @defer
            }
          }
          fragment ComputerErrorField on Computer {
            errorField
          }"#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"computer":{"id":"Computer1"}},"errors":[{"message":"Unexpected error","extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"computers"}},{"message":"Unexpected error","extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"computers"}}],"pending":[{"id":"0","path":["computer"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"errorField":null},"errors":[{"message":"Unexpected error","path":["computer","errorField"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"computers"}}]}],"completed":[{"id":"0"}]}
    computers: query($_v0:ID!){computer(id: $_v0){id errorField}} {"_v0":"Computer1"}
    "#);
}

#[ntex::test]
/// `deferred_fragment_bounds_nullability`: a non-null field that comes back null in a deferred
/// fragment on a list item. The null must stop at the fragment and not touch the primary data.
/// Only the second item has a `nonNullId`, so we also see an item that works.
async fn null_in_a_list_item_fragment_stops_at_the_fragment() {
    let setup = start_full(
        ORGA,
        mocks(suborga_mocks(json!([
            { "__typename": "Organization", "id": "1", "nonNullId": null },
            { "__typename": "Organization", "id": "2", "nonNullId": "2" },
            { "__typename": "Organization", "id": "3", "nonNullId": null }
        ]))),
        "",
    )
    .await;
    // the merged parts can't match the plain response here: without `@defer` the null takes
    // the whole item, with it the null stops at the fragment
    let deferred = setup.send("query { currentUser { activeOrganization { id  suborga { id ...@defer { nonNullId } } } } }",
        None,
    )
    .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"currentUser":{"activeOrganization":{"id":"0","suborga":[{"id":"1"},{"id":"2"},{"id":"3"}]}}},"pending":[{"id":"0","path":["currentUser","activeOrganization","suborga",0]},{"id":"1","path":["currentUser","activeOrganization","suborga",1]},{"id":"2","path":["currentUser","activeOrganization","suborga",2]}],"hasNext":true}
    {"incremental":[{"id":"1","data":{"nonNullId":"2"}}],"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["currentUser","activeOrganization","suborga",0]}]},{"id":"1"},{"id":"2","errors":[{"message":"Cannot return null for non-nullable field.","path":["currentUser","activeOrganization","suborga",2]}]}]}
    orga: query($representations:[_Any!]!){_entities(representations: $representations){...on Organization{nonNullId}}} {"representations":[{"__typename":"Organization","id":"1"},{"__typename":"Organization","id":"2"},{"__typename":"Organization","id":"3"}]}
    orga: query($representations:[_Any!]!){_entities(representations: $representations){...on Organization{suborga{__typename id}}}} {"representations":[{"__typename":"Organization","id":"0"}]}
    user: {currentUser{activeOrganization{__typename id}}}
    "#);
}

#[ntex::test]
/// `errors_on_incremental_responses`: subgraph errors on deferred fields of list items.
async fn list_item_errors_go_to_their_parts() {
    let setup = start_full(
        ORGA,
        canned(vec![
            (
                "user",
                "currentUser",
                json!({ "data": { "currentUser": { "activeOrganization": { "__typename": "Organization", "id": "0" } } } }),
            ),
            (
                "orga",
                "Organization{name}",
                json!({
                    "data": { "_entities": [
                        { "__typename": "Organization", "id": "1", "name": null },
                        { "__typename": "Organization", "id": "2", "name": "A" },
                        { "__typename": "Organization", "id": "3", "name": null }
                    ] },
                    "errors": [
                        { "message": "error orga 1", "path": ["_entities", 0, "name"] },
                        { "message": "error orga 3", "path": ["_entities", 2, "name"] }
                    ]
                }),
            ),
            (
                "orga",
                "suborga{__typename id}",
                json!({
                    "data": { "_entities": [{ "suborga": [
                        { "__typename": "Organization", "id": "1" },
                        { "__typename": "Organization", "id": "2" },
                        { "__typename": "Organization", "id": "3" }
                    ] }] }
                }),
            ),
            // the same query without `@defer`
            (
                "orga",
                "_entities",
                json!({
                    "data": { "_entities": [{ "suborga": [
                        { "__typename": "Organization", "id": "1", "name": null },
                        { "__typename": "Organization", "id": "2", "name": "A" },
                        { "__typename": "Organization", "id": "3", "name": null }
                    ] }] },
                    "errors": [
                        { "message": "error orga 1", "path": ["_entities", 0, "suborga", 0, "name"] },
                        { "message": "error orga 3", "path": ["_entities", 0, "suborga", 2, "name"] }
                    ]
                }),
            ),
        ]),
        "",
    )
    .await;
    let deferred = setup.check("query { currentUser { activeOrganization { id  suborga { id ...@defer { name } } } } }",
        None,
    )
    .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"currentUser":{"activeOrganization":{"id":"0","suborga":[{"id":"1"},{"id":"2"},{"id":"3"}]}}},"pending":[{"id":"0","path":["currentUser","activeOrganization","suborga",0]},{"id":"1","path":["currentUser","activeOrganization","suborga",1]},{"id":"2","path":["currentUser","activeOrganization","suborga",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":null},"errors":[{"message":"Unexpected error","path":["currentUser","activeOrganization","suborga",0,"name"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"orga","affectedPath":"currentUser.activeOrganization.suborga.@"}}]},{"id":"1","data":{"name":"A"}},{"id":"2","data":{"name":null},"errors":[{"message":"Unexpected error","path":["currentUser","activeOrganization","suborga",2,"name"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"orga","affectedPath":"currentUser.activeOrganization.suborga.@"}}]}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    orga: query($representations:[_Any!]!){_entities(representations: $representations){...on Organization{name}}} {"representations":[{"__typename":"Organization","id":"1"},{"__typename":"Organization","id":"2"},{"__typename":"Organization","id":"3"}]}
    orga: query($representations:[_Any!]!){_entities(representations: $representations){...on Organization{suborga{__typename id}}}} {"representations":[{"__typename":"Organization","id":"0"}]}
    user: {currentUser{activeOrganization{__typename id}}}
    "#);
}

#[ntex::test]
/// Discard a nested patch after a non-null deferred field has nulled its parent.
async fn discard_patches_below_a_nulled_parent() {
    let setup = start_full(
        ORGA,
        mocks(json!({
            "user": {
                "query": { "currentUser": {
                    "__typename": "User", "name": "Ada", "id": "1",
                    "activeOrganization": { "__typename": "Organization", "id": "2" }
                } },
                "entities": [
                    { "__typename": "User", "id": "1", "activeOrganization": { "__typename": "Organization", "id": "2" } },
                    { "__typename": "User", "id": "3", "name": "A" }
                ]
            },
            "orga": {
                "entities": [{
                    "__typename": "Organization", "id": "2", "nonNullId": null,
                    "creatorUser": { "__typename": "User", "id": "3" }
                }]
            }
        })),
        "",
    )
    .await;
    let deferred = setup
        .check(
            r#"query {
            currentUser {
                name
                ... @defer {
                    org: activeOrganization {
                        id
                        nonNullId
                        ... @defer {
                            creatorUser {
                                name
                            }
                        }
                    }
                }
            }
        }"#,
            None,
        )
        .await;
    // the creator's name can be asked for before `nonNullId` turns `org` null: it only needs
    // `creatorUser`, so whether that call goes out depends on which `orga` call answers first
    let text: Vec<&str> = deferred
        .text
        .lines()
        .filter(|line| !line.contains("...on User{name}"))
        .collect();
    assert_snapshot!(text.join("\n"), @r#"
    {"data":{"currentUser":{"name":"Ada"}},"pending":[{"id":"0","path":["currentUser"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"org":null}}],"completed":[{"id":"0"}]}
    orga: query($representations:[_Any!]!){_entities(representations: $representations){...on Organization{creatorUser{__typename id}}}} {"representations":[{"__typename":"Organization","id":"2"}]}
    orga: query($representations:[_Any!]!){_entities(representations: $representations){...on Organization{nonNullId}}} {"representations":[{"__typename":"Organization","id":"2"}]}
    user: query($representations:[_Any!]!){_entities(representations: $representations){...on User{org: activeOrganization{__typename id}}}} {"representations":[{"__typename":"User","id":"1"}]}
    user: {currentUser{__typename name id}}
    "#);
}

#[ntex::test]
/// `query_reconstruction`: a mutation with `@defer` and a missing required variable. It must
/// not panic, and answers with a variable error.
async fn missing_required_variable_is_an_error() {
    let setup = start_full(PAYMENTS, None, "").await;
    let deferred = setup
        .send(
            r#"mutation ($userId: ID!) {
            makePayment(userId: $userId) {
              id
              ... @defer {
                paymentStatus {
                  id
                }
              }
            }
          }"#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"{"errors":[{"message":"Variable \"$userId\" has invalid value: Expected a value of non-null type \"ID!\" to be provided.","extensions":{"code":"BAD_USER_INPUT"}}]}"#);
}

/// `b` answers `{data:{userById:null}, errors:[{path:[userById,email]}]}`. The non-null
/// `email` failed, so the whole fragment is dropped.
#[ntex::test]
async fn non_null_error_drops_the_whole_fragment() {
    let a = mock_subgraphs(json!({
        "a": {
            "query": { "user": { "__typename": "User", "id": "VXNlcjox", "name": "User: VXNlcjox" } }
        }
    }));
    let setup = start_with("user-contact", move |req| {
        if req.path == "/b" {
            return respond(json!({
                "data": { "_entities": [null] },
                "errors": [{ "message": "Unexpected Execution Error", "path": ["_entities", 0, "email"] }]
            }));
        }
        a(req)
    })
    .await;
    let parts = setup
        .run(
            r#"
              query GetUser {
                user(id: "1") {
                  name
                  ... @defer(label: "email") { email }
                }
              }
            "#,
            None,
            Plain::Skip,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"user":{"name":"User: VXNlcjox"}},"pending":[{"id":"0","path":["user"],"label":"email"}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Unexpected error","path":["user","email"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"b","affectedPath":"user"}}]}]}
    a: query($_v0:ID!){user(id: $_v0){__typename name id}} {"_v0":"1"}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on User{email}}} {"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    "#);
}

// `Foo.value` resolves to `Baz`, which is `@inaccessible`, so the non-null `value` is nulled
// and that null bubbles up through the deferred fragment.

fn inaccessible_mocks() -> Value {
    let baz = json!({ "__typename": "Baz", "name": "Baz: QmF6OjE=" });
    json!({
        "a": {
            "query": {
                "entity": {
                    "__typename": "Entity",
                    "id": "RW50aXR5OjE=",
                    "wrapper": { "__typename": "Wrapper", "visible": "Wrapper: V3JhcHBlcjoy", "value": baz }
                },
                "entities": [
                    { "__typename": "Entity", "id": "RW50aXR5OjE=", "value": baz },
                    { "__typename": "Entity", "id": "RW50aXR5OjI=", "value": baz },
                    { "__typename": "Entity", "id": "RW50aXR5OjM=", "value": baz }
                ],
                "deep": {
                    "parent": {
                        "visible": "Parent: UGFyZW50OjI=",
                        "child": { "visible": "Child: Q2hpbGQ6Mw==", "value": baz }
                    }
                }
            }
        }
    })
}

#[ntex::test]
async fn inaccessible_type_nulls_the_pending_ancestor() {
    let setup = start("inaccessible", inaccessible_mocks()).await;
    let parts = setup
        .run(
            r#"
              {
                entity {
                  id
                  wrapper {
                    visible
                    ... @defer(label: "hidden") { value { __typename name } }
                  }
                }
              }
            "#,
            None,
            Plain::Skip,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"entity":{"id":"RW50aXR5OjE=","wrapper":{"visible":"Wrapper: V3JhcHBlcjoy"}}},"pending":[{"id":"0","path":["entity","wrapper"],"label":"hidden"}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["entity","wrapper"]}]}]}
    a: {entity{id wrapper{visible value{__typename name}}}}
    "#);
}

#[ntex::test]
async fn inaccessible_type_in_a_list_gives_one_null_patch() {
    let setup = start("inaccessible", inaccessible_mocks()).await;
    let parts = setup
        .run(
            r#"
              {
                entities {
                  id
                  ... @defer(label: "hidden") { value { __typename name } }
                }
              }
            "#,
            None,
            Plain::Skip,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"entities":[{"id":"RW50aXR5OjE="},{"id":"RW50aXR5OjI="},{"id":"RW50aXR5OjM="}]},"pending":[{"id":"0","path":["entities",0],"label":"hidden"},{"id":"1","path":["entities",1],"label":"hidden"},{"id":"2","path":["entities",2],"label":"hidden"}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["entities",0]}]},{"id":"1","errors":[{"message":"Cannot return null for non-nullable field.","path":["entities",1]}]},{"id":"2","errors":[{"message":"Cannot return null for non-nullable field.","path":["entities",2]}]}]}
    a: {entities{id value{__typename name}}}
    "#);
}

#[ntex::test]
async fn null_above_the_pending_path_drops_the_patch() {
    let setup = start("inaccessible", inaccessible_mocks()).await;
    let parts = setup
        .run(
            r#"
              {
                deep {
                  parent {
                    visible
                    child {
                      visible
                      ... @defer(label: "hidden") { value { __typename name } }
                    }
                  }
                }
              }
            "#,
            None,
            Plain::Skip,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"deep":{"parent":{"visible":"Parent: UGFyZW50OjI=","child":{"visible":"Child: Q2hpbGQ6Mw=="}}}},"pending":[{"id":"0","path":["deep","parent","child"],"label":"hidden"}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["deep","parent","child"]}]}]}
    a: {deep{parent{visible child{visible value{__typename name}}}}}
    "#);
}

// ---- authorization ----

#[ntex::test]
/// `unauthenticated_request_defer`: the whole deferred fragment is `@authenticated` and the
/// request isn't.
async fn unauthorized_deferred_field_fails_only_its_part() {
    let setup = start_full(
        ORGA_AUTHENTICATED,
        mocks(json!({
            "orga": {
                "query": { "orga": {
                    "__typename": "Organization", "id": 1, "nonNullId": "1",
                    "creatorUser": { "__typename": "User", "id": 0 }
                } }
            }
        })),
        "jwt:\n  enabled: true\n  require_authentication: false\n  jwks_providers:\n    - source: file\n      path: jwks.rsa512.json\n",
    )
    .await;
    let deferred = setup
        .send(
            "query { orga(id: 1) { id creatorUser { id } ... @defer { nonNullId } } }",
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"orga":{"id":1,"creatorUser":{"id":0}}},"errors":[{"message":"Unauthorized field or type","extensions":{"code":"UNAUTHORIZED_FIELD_OR_TYPE","affectedPath":"orga.nonNullId"}}],"pending":[{"id":"0","path":["orga"]}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["orga"]}]}]}
    orga: query($_v0:ID){orga(id: $_v0){id creatorUser{id}}} {"_v0":1}
    "#);
    // the null from the unauthorized non-null field has to stop at the deferred fragment,
    // `orga` and its other fields are primary data
    assert_eq!(
        deferred.parts[0]["data"]["orga"]["creatorUser"],
        json!({ "id": 0 }),
        "the deferred fragment's null took out primary data"
    );
}

// ---- subgraph down ----

#[ntex::test]
/// `products` is down and answers before the slow primary `accounts` call. Its error has no
/// path, it still goes out with the deferred part its call was for, not with the primary part.
async fn failed_deferred_call_before_the_primary_part_stays_deferred() {
    let subgraphs = TestSubgraphs::builder()
        .with_on_request(|req| {
            (req.path == "/products").then(|| ResponseLike {
                status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                headers: http::HeaderMap::new(),
                body: Some(Bytes::from_static(b"boom")),
            })
        })
        .with_path_delay("/accounts", std::time::Duration::from_millis(500))
        .build()
        .start()
        .await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(
            "supergraph:\n  source: file\n  path: supergraph.graphql\ndefer:\n  enabled: true\n",
        )
        .build()
        .start()
        .await;
    let setup = Setup { subgraphs, router };
    let deferred = setup
        .send(
            r#"{ me { id } ... @defer(label: "products") { topProducts(first: 1) { upc } } }"#,
            None,
        )
        .await;
    assert_eq!(
        deferred.parts[0].get("errors"),
        None,
        "the deferred call's error went out with the primary part"
    );
    assert_snapshot!(deferred.text, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":[],"label":"products"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"topProducts":null},"errors":[{"message":"Unexpected error","extensions":{"code":"SUBREQUEST_MALFORMED_RESPONSE","service":"products"}}]}],"completed":[{"id":"0"}]}
    accounts: {me{id}}
    products: query($_v0:Int){topProducts(first: $_v0){upc}} {"_v0":1}
    "#);
}

#[ntex::test]
/// The deferred `products` call answers before the slow primary `accounts` call, with an error
/// on a key that isn't in the response. It goes out with its part, without that path.
async fn early_deferred_error_with_an_unknown_path_drops_the_path() {
    let subgraphs = TestSubgraphs::builder()
        .with_on_request(|req| {
            (req.path == "/products").then(|| {
                respond(json!({
                    "data": { "topProducts": [{ "upc": "1" }] },
                    "errors": [{ "message": "odd", "path": ["unknown"] }]
                }))
            })?
        })
        .with_path_delay("/accounts", std::time::Duration::from_millis(500))
        .build()
        .start()
        .await;
    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(
            "supergraph:\n  source: file\n  path: supergraph.graphql\ndefer:\n  enabled: true\n",
        )
        .build()
        .start()
        .await;
    let setup = Setup { subgraphs, router };
    let deferred = setup
        .send(
            r#"{ me { id } ... @defer(label: "products") { topProducts(first: 1) { upc } } }"#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":[],"label":"products"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"topProducts":[{"upc":"1"}]},"errors":[{"message":"Unexpected error","extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"products"}}]}],"completed":[{"id":"0"}]}
    accounts: {me{id}}
    products: query($_v0:Int){topProducts(first: $_v0){upc}} {"_v0":1}
    "#);
}

#[ntex::test]
/// `b` answers with a 500. Only the `reviews` part fails, the `name` part still goes out and
/// the stream ends.
async fn failed_subgraph_call_fails_only_its_part() {
    let mut config = user_reviews_mocks();
    config["a"]["entities"] = json!([config["a"]["query"]["user"].clone()]);
    let mocks = mock_subgraphs(config);
    let setup = start_with("user-reviews", move |req: RequestLike| {
        if req.path == "/b" {
            return Some(ResponseLike {
                status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                headers: http::HeaderMap::new(),
                body: Some(Bytes::from_static(b"boom")),
            });
        }
        mocks(req)
    })
    .await;
    let deferred = setup
        .send(
            r#"query { user(id: "VXNlcjox") { id ... @defer(label: "name") { name } ... @defer(label: "reviews") { reviews { title } } } }"#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"user":{"id":"VXNlcjox"}},"pending":[{"id":"0","path":["user"],"label":"name"},{"id":"1","path":["user"],"label":"reviews"}],"hasNext":true}
    {"completed":[{"id":"1","errors":[{"message":"Unexpected error","extensions":{"code":"SUBREQUEST_MALFORMED_RESPONSE","service":"b","affectedPath":"user"}}]}]}
    {"incremental":[{"id":"0","data":{"name":"User: VXNlcjox"}}],"completed":[{"id":"0"}]}
    a: query($_v0:ID!){user(id: $_v0){__typename id}} {"_v0":"VXNlcjox"}
    a: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{title}}}} {"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    "#);
    assert_eq!(
        deferred.parts.last().unwrap()["hasNext"],
        json!(false),
        "the stream didn't end"
    );
}
