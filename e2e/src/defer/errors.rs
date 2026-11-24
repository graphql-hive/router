//! Subgraph errors and nulls in deferred fragments: an error goes out with the part that owns
//! it, a null stops at the fragment, and a fragment under a nulled parent is never sent.

use insta::assert_snapshot;
use serde_json::{json, Value};

use super::*;

#[ntex::test]
/// The `name` error comes from the deferred `name` call and goes out with its patch.
async fn deferred_field_error_keeps_its_patch() {
    let setup = start_full(
        SUPERGRAPH,
        canned(vec![
            (
                "accounts",
                "_entities",
                json!({
                    "data": { "_entities": [{ "__typename": "User", "id": "1", "name": null }] },
                    "errors": [{ "message": "error user 1", "path": ["_entities", 0, "name"] }]
                }),
            ),
            // the same query without `@defer`
            (
                "accounts",
                "me{id name}",
                json!({
                    "data": { "me": { "__typename": "User", "id": "1", "name": null } },
                    "errors": [{ "message": "error user 1", "path": ["me", "name"] }]
                }),
            ),
            (
                "accounts",
                "me{",
                json!({ "data": { "me": { "__typename": "User", "id": "1" } } }),
            ),
        ]),
        "",
    )
    .await;
    let deferred = setup
        .check("query { me { id ...@defer { name } } }", None)
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":null},"errors":[{"message":"Unexpected error","path":["me","name"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"accounts","affectedPath":"me"}}]}],"completed":[{"id":"0"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id}}
    "#);
}

#[ntex::test]
/// `Warehouse` isn't an entity, so the deferred `location` comes with the primary fetch, and
/// so does its error. The error still belongs in the deferred part.
async fn primary_fetch_error_goes_to_the_deferred_part() {
    let setup = start_full(
        SUPERGRAPH,
        canned(vec![(
            "inventory",
            "warehouse",
            json!({
                "data": { "warehouse": { "id": "1" } },
                "errors": [{
                    "message": "Error field",
                    "locations": [{ "line": 1, "column": 93 }],
                    "path": ["warehouse", "location"]
                }]
            }),
        )]),
        "",
    )
    .await;
    let deferred = setup
        .check(
            r#"query {
            warehouse(id: "1") {
              id
              ...WarehouseLocation @defer
            }
          }
          fragment WarehouseLocation on Warehouse {
            location
          }"#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"warehouse":{"id":"1"}},"pending":[{"id":"0","path":["warehouse"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"location":null},"errors":[{"message":"Unexpected error","locations":[{"line":1,"column":93}],"path":["warehouse","location"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"inventory"}}]}],"completed":[{"id":"0"}]}
    inventory: query($_v0:ID!){warehouse(id: $_v0){id location}} {"_v0":"1"}
    "#);
}

#[ntex::test]
/// Subgraph error paths that aren't in the response are dropped or cut back to the part that is.
async fn invalid_error_paths_are_dropped_or_cut_back() {
    let setup = start_full(
        SUPERGRAPH,
        canned(vec![(
            "inventory",
            "warehouse",
            json!({
                "data": { "warehouse": { "id": "1" } },
                "errors": [
                    { "message": "Subgraph error with invalid path", "path": ["invalid", "path"] },
                    { "message": "Subgraph error with partially valid path", "path": ["me", "invalidSubpath"] },
                    { "message": "Error field", "path": ["warehouse", "location", "locationSubpath"] }
                ]
            }),
        )]),
        "",
    )
    .await;
    let deferred = setup
        .check(
            r#"query {
            warehouse(id: "1") {
              id
              ...WarehouseLocation @defer
            }
          }
          fragment WarehouseLocation on Warehouse {
            location
          }"#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"warehouse":{"id":"1"}},"errors":[{"message":"Unexpected error","extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"inventory"}},{"message":"Unexpected error","extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"inventory"}}],"pending":[{"id":"0","path":["warehouse"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"location":null},"errors":[{"message":"Unexpected error","path":["warehouse","location"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"inventory"}}]}],"completed":[{"id":"0"}]}
    inventory: query($_v0:ID!){warehouse(id: $_v0){id location}} {"_v0":"1"}
    "#);
}

#[ntex::test]
/// A non-null field that comes back null in a deferred fragment on a list item. The null must
/// stop at the fragment and not touch the primary data. Only the second item has a `sku`, so we
/// also see an item that works.
async fn null_in_a_list_item_fragment_stops_at_the_fragment() {
    let setup = start_full(
        SUPERGRAPH,
        mocks(related_products_mocks(json!([
            { "__typename": "Product", "id": "1", "sku": null },
            { "__typename": "Product", "id": "2", "sku": "SKU-2" },
            { "__typename": "Product", "id": "3", "sku": null }
        ]))),
        "",
    )
    .await;
    // the merged parts can't match the plain response here: without `@defer` the null takes
    // the whole item, with it the null stops at the fragment
    let deferred = setup
        .send(
            "query { me { favoriteProduct { id relatedProducts { id ...@defer { sku } } } } }",
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"me":{"favoriteProduct":{"id":"0","relatedProducts":[{"id":"1"},{"id":"2"},{"id":"3"}]}}},"pending":[{"id":"0","path":["me","favoriteProduct","relatedProducts",0]},{"id":"1","path":["me","favoriteProduct","relatedProducts",1]},{"id":"2","path":["me","favoriteProduct","relatedProducts",2]}],"hasNext":true}
    {"incremental":[{"id":"1","data":{"sku":"SKU-2"}}],"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["me","favoriteProduct","relatedProducts",0]}]},{"id":"1"},{"id":"2","errors":[{"message":"Cannot return null for non-nullable field.","path":["me","favoriteProduct","relatedProducts",2]}]}]}
    accounts: {me{favoriteProduct{__typename id}}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{relatedProducts{__typename id}}}} {"representations":[{"__typename":"Product","id":"0"}]}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{sku}}} {"representations":[{"__typename":"Product","id":"1"},{"__typename":"Product","id":"2"},{"__typename":"Product","id":"3"}]}
    "#);
}

#[ntex::test]
/// Subgraph errors on deferred fields of list items.
async fn list_item_errors_go_to_their_parts() {
    let setup = start_full(
        SUPERGRAPH,
        canned(vec![
            (
                "accounts",
                "me{",
                json!({ "data": { "me": { "favoriteProduct": { "__typename": "Product", "id": "0" } } } }),
            ),
            (
                "products",
                "Product{name}",
                json!({
                    "data": { "_entities": [
                        { "__typename": "Product", "id": "1", "name": null },
                        { "__typename": "Product", "id": "2", "name": "Chair" },
                        { "__typename": "Product", "id": "3", "name": null }
                    ] },
                    "errors": [
                        { "message": "error product 1", "path": ["_entities", 0, "name"] },
                        { "message": "error product 3", "path": ["_entities", 2, "name"] }
                    ]
                }),
            ),
            (
                "products",
                "relatedProducts{__typename id}",
                json!({
                    "data": { "_entities": [{ "relatedProducts": [
                        { "__typename": "Product", "id": "1" },
                        { "__typename": "Product", "id": "2" },
                        { "__typename": "Product", "id": "3" }
                    ] }] }
                }),
            ),
            // the same query without `@defer`
            (
                "products",
                "_entities",
                json!({
                    "data": { "_entities": [{ "relatedProducts": [
                        { "__typename": "Product", "id": "1", "name": null },
                        { "__typename": "Product", "id": "2", "name": "Chair" },
                        { "__typename": "Product", "id": "3", "name": null }
                    ] }] },
                    "errors": [
                        { "message": "error product 1", "path": ["_entities", 0, "relatedProducts", 0, "name"] },
                        { "message": "error product 3", "path": ["_entities", 0, "relatedProducts", 2, "name"] }
                    ]
                }),
            ),
        ]),
        "",
    )
    .await;
    let deferred = setup
        .check(
            "query { me { favoriteProduct { id relatedProducts { id ...@defer { name } } } } }",
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"me":{"favoriteProduct":{"id":"0","relatedProducts":[{"id":"1"},{"id":"2"},{"id":"3"}]}}},"pending":[{"id":"0","path":["me","favoriteProduct","relatedProducts",0]},{"id":"1","path":["me","favoriteProduct","relatedProducts",1]},{"id":"2","path":["me","favoriteProduct","relatedProducts",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":null},"errors":[{"message":"Unexpected error","path":["me","favoriteProduct","relatedProducts",0,"name"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"products","affectedPath":"me.favoriteProduct.relatedProducts.@"}}]},{"id":"1","data":{"name":"Chair"}},{"id":"2","data":{"name":null},"errors":[{"message":"Unexpected error","path":["me","favoriteProduct","relatedProducts",2,"name"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"products","affectedPath":"me.favoriteProduct.relatedProducts.@"}}]}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: {me{favoriteProduct{__typename id}}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","id":"1"},{"__typename":"Product","id":"2"},{"__typename":"Product","id":"3"}]}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{relatedProducts{__typename id}}}} {"representations":[{"__typename":"Product","id":"0"}]}
    "#);
}

#[ntex::test]
/// Discard a nested patch after a non-null deferred field has nulled its parent.
async fn discard_patches_below_a_nulled_parent() {
    let setup = start_full(
        SUPERGRAPH,
        mocks(json!({
            "accounts": {
                "query": { "me": {
                    "__typename": "User", "name": "Ada", "id": "1",
                    "favoriteProduct": { "__typename": "Product", "id": "2" }
                } },
                "entities": [
                    { "__typename": "User", "id": "1", "favoriteProduct": { "__typename": "Product", "id": "2" } },
                    { "__typename": "User", "id": "3", "name": "Grace" }
                ]
            },
            "products": {
                "entities": [{
                    "__typename": "Product", "id": "2", "sku": null,
                    "createdBy": { "__typename": "User", "id": "3" }
                }]
            }
        })),
        "",
    )
    .await;
    let deferred = setup
        .check(
            r#"query {
            me {
                name
                ... @defer {
                    favorite: favoriteProduct {
                        id
                        sku
                        ... @defer {
                            createdBy {
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
    // the creator's name can be asked for before `sku` turns `favorite` null: it only needs
    // `createdBy`, so whether that call goes out depends on which `products` call answers first
    let text: Vec<&str> = deferred
        .text
        .lines()
        .filter(|line| !line.contains("...on User{name}"))
        .collect();
    assert_snapshot!(text.join("\n"), @r#"
    {"data":{"me":{"name":"Ada"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"favorite":null}}],"completed":[{"id":"0"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{favorite: favoriteProduct{__typename id}}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename name id}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{createdBy{__typename id}}}} {"representations":[{"__typename":"Product","id":"2"}]}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{sku}}} {"representations":[{"__typename":"Product","id":"2"}]}
    "#);
}

#[ntex::test]
/// A mutation with `@defer` and a missing required variable. It must not panic, and answers
/// with a variable error.
async fn missing_required_variable_is_an_error() {
    let setup = start_full(SUPERGRAPH, None, "").await;
    let deferred = setup
        .send(
            r#"mutation ($name: String!) {
            createProduct(name: $name) {
              id
              ... @defer {
                price
              }
            }
          }"#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"{"errors":[{"message":"Variable \"$name\" has invalid value: Expected a value of non-null type \"String!\" to be provided.","extensions":{"code":"BAD_USER_INPUT"}}]}"#);
}

/// The deferred `email` call answers with a null entity and an error on the non-null `email`,
/// so the whole fragment is dropped.
#[ntex::test]
async fn non_null_error_drops_the_whole_fragment() {
    let accounts = mock_subgraphs(json!({
        "accounts": {
            "query": { "user": { "__typename": "User", "id": "1", "name": "Ada" } }
        }
    }));
    let setup = start_with(move |req| {
        let entities_call = req
            .body
            .as_ref()
            .is_some_and(|body| String::from_utf8_lossy(body).contains("_entities"));
        if entities_call {
            return respond(json!({
                "data": { "_entities": [null] },
                "errors": [{ "message": "Unexpected Execution Error", "path": ["_entities", 0, "email"] }]
            }));
        }
        accounts(req)
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
    {"data":{"user":{"name":"Ada"}},"pending":[{"id":"0","path":["user"],"label":"email"}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Unexpected error","path":["user","email"],"extensions":{"code":"DOWNSTREAM_SERVICE_ERROR","service":"accounts","affectedPath":"user"}}]}]}
    accounts: query($_v0:ID!){user(id: $_v0){__typename name id}} {"_v0":"1"}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{email}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

// The badge resolves to a `SaleBadge`, which is `@inaccessible`, so the non-null `badge` is
// nulled and that null bubbles up through the deferred fragment.

fn inaccessible_mocks() -> Value {
    let sale = json!({ "__typename": "SaleBadge", "label": "50% off" });
    json!({
        "products": {
            "query": {
                "category": {
                    "__typename": "Category",
                    "id": "1",
                    "banner": { "__typename": "Banner", "title": "Summer", "badge": sale }
                },
                "categories": [
                    { "__typename": "Category", "id": "1", "badge": sale },
                    { "__typename": "Category", "id": "2", "badge": sale },
                    { "__typename": "Category", "id": "3", "badge": sale }
                ],
                "catalog": {
                    "shelf": {
                        "title": "Garden",
                        "section": { "title": "Tools", "badge": sale }
                    }
                }
            }
        }
    })
}

#[ntex::test]
async fn inaccessible_type_nulls_the_pending_ancestor() {
    let setup = start(inaccessible_mocks()).await;
    let parts = setup
        .run(
            r#"
              {
                category {
                  id
                  banner {
                    title
                    ... @defer(label: "hidden") { badge { __typename label } }
                  }
                }
              }
            "#,
            None,
            Plain::Skip,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"category":{"id":"1","banner":{"title":"Summer"}}},"pending":[{"id":"0","path":["category","banner"],"label":"hidden"}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["category","banner"]}]}]}
    products: {category{id banner{title badge{__typename label}}}}
    "#);
}

#[ntex::test]
async fn inaccessible_type_in_a_list_gives_one_null_patch() {
    let setup = start(inaccessible_mocks()).await;
    let parts = setup
        .run(
            r#"
              {
                categories {
                  id
                  ... @defer(label: "hidden") { badge { __typename label } }
                }
              }
            "#,
            None,
            Plain::Skip,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"categories":[{"id":"1"},{"id":"2"},{"id":"3"}]},"pending":[{"id":"0","path":["categories",0],"label":"hidden"},{"id":"1","path":["categories",1],"label":"hidden"},{"id":"2","path":["categories",2],"label":"hidden"}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["categories",0]}]},{"id":"1","errors":[{"message":"Cannot return null for non-nullable field.","path":["categories",1]}]},{"id":"2","errors":[{"message":"Cannot return null for non-nullable field.","path":["categories",2]}]}]}
    products: {categories{id badge{__typename label}}}
    "#);
}

#[ntex::test]
async fn null_above_the_pending_path_drops_the_patch() {
    let setup = start(inaccessible_mocks()).await;
    let parts = setup
        .run(
            r#"
              {
                catalog {
                  shelf {
                    title
                    section {
                      title
                      ... @defer(label: "hidden") { badge { __typename label } }
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
    {"data":{"catalog":{"shelf":{"title":"Garden","section":{"title":"Tools"}}}},"pending":[{"id":"0","path":["catalog","shelf","section"],"label":"hidden"}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["catalog","shelf","section"]}]}]}
    products: {catalog{shelf{title section{title badge{__typename label}}}}}
    "#);
}

// ---- authorization ----

#[ntex::test]
/// The whole deferred fragment is `@authenticated` and the request isn't.
async fn unauthorized_deferred_field_fails_only_its_part() {
    let setup = start_full(
        SUPERGRAPH,
        mocks(json!({
            "products": {
                "query": { "product": {
                    "__typename": "Product", "id": "1", "cost": 9.5,
                    "createdBy": { "__typename": "User", "id": "2" }
                } }
            }
        })),
        "jwt:\n  enabled: true\n  require_authentication: false\n  jwks_providers:\n    - source: file\n      path: jwks.rsa512.json\n",
    )
    .await;
    let deferred = setup
        .send(
            r#"query { product(id: "1") { id createdBy { id } ... @defer { cost } } }"#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"product":{"id":"1","createdBy":{"id":"2"}}},"errors":[{"message":"Unauthorized field or type","extensions":{"code":"UNAUTHORIZED_FIELD_OR_TYPE","affectedPath":"product.cost"}}],"pending":[{"id":"0","path":["product"]}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["product"]}]}]}
    products: query($_v0:ID!){product(id: $_v0){id createdBy{id}}} {"_v0":"1"}
    "#);
    // the null from the unauthorized non-null field has to stop at the deferred fragment,
    // `product` and its other fields are primary data
    assert_eq!(
        deferred.parts[0]["data"]["product"]["createdBy"],
        json!({ "id": "2" }),
        "the deferred fragment's null took out primary data"
    );
}

// ---- subgraph down ----

#[ntex::test]
/// `products` is down and answers before the slow primary `accounts` call. Its error has no
/// path, it still goes out with the deferred part its call was for, not with the primary part.
async fn failed_deferred_call_before_the_primary_part_stays_deferred() {
    let setup = start_on(
        "supergraph.graphql",
        TestSubgraphs::builder()
            .with_on_request(|req| {
                (req.path == "/products").then(|| ResponseLike {
                    status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    headers: http::HeaderMap::new(),
                    body: Some(Bytes::from_static(b"boom")),
                })
            })
            .with_path_delay("/accounts", std::time::Duration::from_millis(500)),
        "",
    )
    .await;
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
    let setup = start_on(
        "supergraph.graphql",
        TestSubgraphs::builder()
            .with_on_request(|req| {
                (req.path == "/products").then(|| {
                    respond(json!({
                        "data": { "topProducts": [{ "upc": "1" }] },
                        "errors": [{ "message": "odd", "path": ["unknown"] }]
                    }))
                })?
            })
            .with_path_delay("/accounts", std::time::Duration::from_millis(500)),
        "",
    )
    .await;
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
/// `reviews` answers with a 500. Only the `reviews` part fails, the `name` part still goes out
/// and the stream ends.
async fn failed_subgraph_call_fails_only_its_part() {
    let mut config = user_reviews_mocks();
    config["accounts"]["entities"] = json!([config["accounts"]["query"]["user"].clone()]);
    let mocks = mock_subgraphs(config);
    let setup = start_with(move |req: RequestLike| {
        if req.path == "/reviews" {
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
            r#"query { user(id: "1") { id ... @defer(label: "name") { name } ... @defer(label: "reviews") { reviews { title } } } }"#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"user":{"id":"1"}},"pending":[{"id":"0","path":["user"],"label":"name"},{"id":"1","path":["user"],"label":"reviews"}],"hasNext":true}
    {"completed":[{"id":"1","errors":[{"message":"Unexpected error","extensions":{"code":"SUBREQUEST_MALFORMED_RESPONSE","service":"reviews","affectedPath":"user"}}]}]}
    {"incremental":[{"id":"0","data":{"name":"Ada"}}],"completed":[{"id":"0"}]}
    accounts: query($_v0:ID!){user(id: $_v0){__typename id}} {"_v0":"1"}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{title}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
    assert_eq!(
        deferred.parts.last().unwrap()["hasNext"],
        json!(false),
        "the stream didn't end"
    );
}
