//! Where the parts land: the root, list items, nested and sibling defers, abstract types and
//! root `__typename`.

use insta::assert_snapshot;
use serde_json::{json, Value};

use super::*;

const USER_REVIEWS_QUERY: &str = r#"
  query GetUser {
    user(id: "1") {
      name
      ... @defer(label: "reviews") {
        reviews { title body }
      }
    }
  }
"#;

#[ntex::test]
async fn single_deferred_fragment() {
    let setup = start("user-reviews", user_reviews_mocks()).await;
    assert_snapshot!(setup.run(USER_REVIEWS_QUERY, None, Plain::Stripped).await, @r#"
    {"data":{"user":{"name":"User: VXNlcjox"}},"pending":[{"id":"0","path":["user"],"label":"reviews"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"reviews":[{"title":"Review: UmV2aWV3OjE=","body":"Review: UmV2aWV3OjE="},{"title":"Review: UmV2aWV3OjI=","body":"Review: UmV2aWV3OjI="},{"title":"Review: UmV2aWV3OjM=","body":"Review: UmV2aWV3OjM="}]}}],"completed":[{"id":"0"}]}
    a: query($_v0:ID!){user(id: $_v0){__typename name id}} {"_v0":"1"}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{title body}}}} {"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    "#);
}

// User `name` from `a`, `email` and `address` from `b`.

fn user_contact_mocks() -> Value {
    json!({
        "a": {
            "query": { "user": { "__typename": "User", "id": "VXNlcjox", "name": "User: VXNlcjox" } }
        },
        "b": {
            "entities": [{ "__typename": "User", "id": "VXNlcjox", "email": "User: VXNlcjox", "address": "User: VXNlcjox" }]
        }
    })
}

#[ntex::test]
async fn nested_defers_go_out_in_order() {
    let setup = start("user-contact", user_contact_mocks()).await;
    let parts = setup
        .run(
            r#"
              {
                user(id: "1") {
                  name
                  ... @defer(label: "outer") {
                    email
                    ... @defer(label: "inner") { address }
                  }
                }
              }
            "#,
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"user":{"name":"User: VXNlcjox"}},"pending":[{"id":"0","path":["user"],"label":"outer"}],"hasNext":true}
    {"pending":[{"id":"1","path":["user"],"label":"inner"}],"incremental":[{"id":"0","data":{"email":"User: VXNlcjox"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"address":"User: VXNlcjox"}}],"completed":[{"id":"1"}]}
    a: query($_v0:ID!){user(id: $_v0){__typename name id}} {"_v0":"1"}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on User{address}}} {"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on User{email}}} {"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    "#);
}

#[ntex::test]
async fn unlabeled_inner_defer() {
    let setup = start("user-contact", user_contact_mocks()).await;
    let parts = setup
        .run(
            r#"
              {
                user(id: "1") {
                  name
                  ... @defer(label: "outer") {
                    email
                    ... @defer { address }
                  }
                }
              }
            "#,
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"user":{"name":"User: VXNlcjox"}},"pending":[{"id":"0","path":["user"],"label":"outer"}],"hasNext":true}
    {"pending":[{"id":"1","path":["user"]}],"incremental":[{"id":"0","data":{"email":"User: VXNlcjox"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"address":"User: VXNlcjox"}}],"completed":[{"id":"1"}]}
    a: query($_v0:ID!){user(id: $_v0){__typename name id}} {"_v0":"1"}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on User{address}}} {"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on User{email}}} {"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    "#);
}

const TWO_SIBLINGS_QUERY: &str = r#"
  {
    user(id: "1") {
      name
      ... @defer(label: "contact") { email }
      ... @defer(label: "location") { email address }
    }
  }
"#;

#[ntex::test]
async fn sibling_defers_with_overlapping_fields() {
    let setup = start("user-contact", user_contact_mocks()).await;
    assert_snapshot!(setup.run(TWO_SIBLINGS_QUERY, None, Plain::Stripped).await, @r#"
    {"data":{"user":{"name":"User: VXNlcjox"}},"pending":[{"id":"0","path":["user"],"label":"contact"},{"id":"1","path":["user"],"label":"location"}],"hasNext":true}
    {"incremental":[{"id":"1","data":{"email":"User: VXNlcjox","address":"User: VXNlcjox"}}],"completed":[{"id":"1"}]}
    {"incremental":[{"id":"0","data":{"email":"User: VXNlcjox"}}],"completed":[{"id":"0"}]}
    a: query($_v0:ID!){user(id: $_v0){__typename name id}} {"_v0":"1"}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on User{email address}}} {"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on User{email}}} {"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    "#);
}

#[ntex::test]
/// `defer_empty_primary_response`: the only field under `me` is deferred.
async fn empty_object_stays_in_the_initial_part() {
    let setup = demo().await;
    let query = r#"{ me { ...@defer(label: "name") { name } } }"#;
    let deferred = setup.check(query, None).await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"me":{}},"pending":[{"id":"0","path":["me"],"label":"name"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"0"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id}}
    "#);
    let (content_type, body) = setup.send_raw(query, None, LEGACY).await;
    assert_snapshot!(format!("{content_type}\n{body}"), @r#"
    multipart/mixed;boundary="-";deferSpec=20220824
    ---
    Content-Type: application/json

    {"data":{"me":{}},"hasNext":true}
    ---
    Content-Type: application/json

    {"incremental":[{"data":{"name":"Uri Goldshtein"},"path":["me"],"label":"name"}],"hasNext":false}
    -----
    "#);
}

#[ntex::test]
/// `defer_path_in_array`: a deferred fragment under a list.
async fn each_list_item_gets_its_own_part() {
    let setup = demo().await;
    let query = r#"{
        me {
            reviews {
                id
                author {
                    id
                    ... @defer(label: "author name") {
                        name
                    }
                }
            }
        }
    }"#;
    let deferred = setup.check(query, None).await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"me":{"reviews":[{"id":"1","author":{"id":"1"}},{"id":"2","author":{"id":"1"}}]}},"pending":[{"id":"0","path":["me","reviews",0,"author"],"label":"author name"},{"id":"1","path":["me","reviews",1,"author"],"label":"author name"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein"}},{"id":"1","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"0"},{"id":"1"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id}}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id author{__typename id}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
    let (content_type, body) = setup.send_raw(query, None, LEGACY).await;
    assert_snapshot!(format!("{content_type}\n{body}"), @r#"
    multipart/mixed;boundary="-";deferSpec=20220824
    ---
    Content-Type: application/json

    {"data":{"me":{"reviews":[{"id":"1","author":{"id":"1"}},{"id":"2","author":{"id":"1"}}]}},"hasNext":true}
    ---
    Content-Type: application/json

    {"incremental":[{"data":{"name":"Uri Goldshtein"},"path":["me","reviews",0,"author"],"label":"author name"},{"data":{"name":"Uri Goldshtein"},"path":["me","reviews",1,"author"],"label":"author name"}],"hasNext":false}
    -----
    "#);
}

fn roots_mocks() -> Value {
    let users = json!([
        { "__typename": "User", "id": "VXNlcjox", "name": "User: VXNlcjox" },
        { "__typename": "User", "id": "VXNlcjoy", "name": "User: VXNlcjoy" },
        { "__typename": "User", "id": "VXNlcjoz", "name": "User: VXNlcjoz" }
    ]);
    let orders = json!([
        { "__typename": "Order", "id": "T3JkZXI6MQ==", "total": 123.456 },
        { "__typename": "Order", "id": "T3JkZXI6Mg==", "total": 123.456 },
        { "__typename": "Order", "id": "T3JkZXI6Mw==", "total": 123.456 }
    ]);
    json!({
        "a": {
            "query": {
                "products": [
                    { "__typename": "Product", "id": "UHJvZHVjdDox", "name": "Product: UHJvZHVjdDox" },
                    { "__typename": "Product", "id": "UHJvZHVjdDoy", "name": "Product: UHJvZHVjdDoy" },
                    { "__typename": "Product", "id": "UHJvZHVjdDoz", "name": "Product: UHJvZHVjdDoz" }
                ]
            }
        },
        "b": {
            "query": { "users": users },
            "entities": users
        },
        "c": {
            "query": { "orders": orders },
            "entities": orders
        }
    })
}

#[ntex::test]
async fn defer_under_the_second_root_field() {
    let setup = start("roots", roots_mocks()).await;
    let parts = setup
        .run(
            "query t { products { id name } users { __typename ... @defer { id name } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"products":[{"id":"UHJvZHVjdDox","name":"Product: UHJvZHVjdDox"},{"id":"UHJvZHVjdDoy","name":"Product: UHJvZHVjdDoy"},{"id":"UHJvZHVjdDoz","name":"Product: UHJvZHVjdDoz"}],"users":[{"__typename":"User"},{"__typename":"User"},{"__typename":"User"}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"id":"VXNlcjox","name":"User: VXNlcjox"}},{"id":"1","data":{"id":"VXNlcjoy","name":"User: VXNlcjoy"}},{"id":"2","data":{"id":"VXNlcjoz","name":"User: VXNlcjoz"}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    a: {products{id name}}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on User{id name}}} {"representations":[{"__typename":"User","id":"VXNlcjox"},{"__typename":"User","id":"VXNlcjoy"},{"__typename":"User","id":"VXNlcjoz"}]}
    b: {users{__typename id}}
    "#);
}

#[ntex::test]
async fn defer_under_the_third_root_field() {
    let setup = start("roots", roots_mocks()).await;
    let parts = setup
        .run(
            "query t { products { id name } users { id name } orders { __typename ... @defer { id total } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"products":[{"id":"UHJvZHVjdDox","name":"Product: UHJvZHVjdDox"},{"id":"UHJvZHVjdDoy","name":"Product: UHJvZHVjdDoy"},{"id":"UHJvZHVjdDoz","name":"Product: UHJvZHVjdDoz"}],"users":[{"id":"VXNlcjox","name":"User: VXNlcjox"},{"id":"VXNlcjoy","name":"User: VXNlcjoy"},{"id":"VXNlcjoz","name":"User: VXNlcjoz"}],"orders":[{"__typename":"Order"},{"__typename":"Order"},{"__typename":"Order"}]},"pending":[{"id":"0","path":["orders",0]},{"id":"1","path":["orders",1]},{"id":"2","path":["orders",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"id":"T3JkZXI6MQ==","total":123.456}},{"id":"1","data":{"id":"T3JkZXI6Mg==","total":123.456}},{"id":"2","data":{"id":"T3JkZXI6Mw==","total":123.456}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    a: {products{id name}}
    b: {users{id name}}
    c: query($representations:[_Any!]!){_entities(representations: $representations){...on Order{id total}}} {"representations":[{"__typename":"Order","id":"T3JkZXI6MQ=="},{"__typename":"Order","id":"T3JkZXI6Mg=="},{"__typename":"Order","id":"T3JkZXI6Mw=="}]}
    c: {orders{__typename id}}
    "#);
}

#[ntex::test]
async fn path_crosses_intermediate_lists() {
    let setup = start(
        "intermediate-lists",
        json!({
            "a": {
                "query": {
                    "users": {
                        "__typename": "DefaultUserConnection",
                        "nodes": [
                            { "reviews": [
                                { "product": { "name": "Table", "dimension": { "height": 30.0 } } },
                                { "product": { "name": "Chair", "dimension": { "height": 40.0 } } }
                            ] },
                            { "reviews": [
                                { "product": { "name": "Couch", "dimension": { "height": 50.0 } } }
                            ] }
                        ]
                    }
                }
            }
        }),
    )
    .await;
    let parts = setup
        .run(
            r#"
              {
                users {
                  nodes {
                    reviews {
                      product {
                        name
                        ... @defer(label: "dimension") { dimension { height } }
                      }
                    }
                  }
                }
              }
            "#,
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":{"nodes":[{"reviews":[{"product":{"name":"Table"}},{"product":{"name":"Chair"}}]},{"reviews":[{"product":{"name":"Couch"}}]}]}},"pending":[{"id":"0","path":["users","nodes",0,"reviews",0,"product"],"label":"dimension"},{"id":"1","path":["users","nodes",0,"reviews",1,"product"],"label":"dimension"},{"id":"2","path":["users","nodes",1,"reviews",0,"product"],"label":"dimension"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"dimension":{"height":30.0}}},{"id":"1","data":{"dimension":{"height":40.0}}},{"id":"2","data":{"dimension":{"height":50.0}}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    a: {users{nodes{reviews{product{name dimension{height}}}}}}
    "#);
}

// ---- abstract types ----

#[ntex::test]
/// `reconstruct_deferred_query_under_interface`: `... on Account @defer` under an interface.
async fn defer_on_an_object_under_an_interface() {
    let setup = start_full(
        IDENTITY,
        mocks(json!({
            "user": {
                "query": { "me": {
                    "__typename": "User", "id": 0, "fullName": "A",
                    "memberships": [{ "permission": "USER", "account": { "__typename": "Account", "id": 1, "name": "B" } }]
                } },
                "entities": [{ "__typename": "Account", "id": 1, "name": "B" }]
            }
        })),
        "",
    )
    .await;
    let deferred = setup
        .check(
            r#"query {
            me {
              ... on User {
                id
                fullName
                memberships {
                  permission
                  account {
                    ... on Account @defer {
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
    assert_snapshot!(deferred.text, @r#"
    {"data":{"me":{"id":0,"fullName":"A","memberships":[{"permission":"USER","account":{}}]}},"pending":[{"id":"0","path":["me","memberships",0,"account"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"B"}}],"completed":[{"id":"0"}]}
    user: query($representations:[_Any!]!){_entities(representations: $representations){...on Account{name}}} {"representations":[{"__typename":"Account","id":1}]}
    user: {me{__typename ...on User{id fullName memberships{permission account{__typename id}}}}}
    "#);
}

#[ntex::test]
async fn defer_under_a_type_condition_on_an_abstract_parent() {
    let edge = |cursor: &str, id: &str| {
        json!({
            "cursor": format!("ConnectorDeviceEdge: {cursor}"),
            "node": { "__typename": "ConnectorDevice", "id": id, "name": format!("ConnectorDevice: {id}") }
        })
    };
    let setup = start(
        "connector",
        json!({
            "a": {
                "query": {
                    "node": {
                        "__typename": "Connector",
                        "id": "Q29ubmVjdG9yOjE=",
                        "system": "Connector: Q29ubmVjdG9yOjE=",
                        "version": "Connector: Q29ubmVjdG9yOjE=",
                        "vendor": "Connector: Q29ubmVjdG9yOjE=",
                        "description": "Connector: Q29ubmVjdG9yOjE="
                    }
                },
                "entities": [{
                    "__typename": "Connector",
                    "id": "Q29ubmVjdG9yOjE=",
                    "system": "Connector: Q29ubmVjdG9yOjE=",
                    "description": "Connector: Q29ubmVjdG9yOjE="
                }]
            },
            "b": {
                "entities": [{
                    "__typename": "Connector",
                    "id": "Q29ubmVjdG9yOjE=",
                    "devices(first: 10)": {
                        "totalCount": 123,
                        "edges": [
                            edge("Q29ubmVjdG9yRGV2aWNlRWRnZToy", "Q29ubmVjdG9yRGV2aWNlOjU="),
                            edge("Q29ubmVjdG9yRGV2aWNlRWRnZToz", "Q29ubmVjdG9yRGV2aWNlOjY="),
                            edge("Q29ubmVjdG9yRGV2aWNlRWRnZTo0", "Q29ubmVjdG9yRGV2aWNlOjc=")
                        ]
                    }
                }]
            }
        }),
    )
    .await;
    let parts = setup
        .run(
            r#"
              query ConnectorDetailsQuery($id: ID!) {
                node(id: $id) {
                  __typename
                  id
                  ... on Connector @defer(label: "Yrs") { system description }
                  ...ConnectorDetailsHeaderFragment
                  ...ConnectorDevicesFragment @defer(label: "testing")
                }
              }

              fragment ConnectorDetailsHeaderFragment on Connector { system version vendor }

              fragment ConnectorDevicesFragment on Connector {
                id
                devices(first: 10) {
                  edges {
                    node { ... on ConnectorDevice { id name __typename } }
                    cursor
                  }
                  totalCount
                }
              }
            "#,
            Some(json!({ "id": "Q29ubmVjdG9yOjE=" })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"node":{"__typename":"Connector","id":"Q29ubmVjdG9yOjE=","system":"Connector: Q29ubmVjdG9yOjE=","version":"Connector: Q29ubmVjdG9yOjE=","vendor":"Connector: Q29ubmVjdG9yOjE="}},"pending":[{"id":"0","path":["node"],"label":"Yrs"},{"id":"1","path":["node"],"label":"testing"}],"hasNext":true}
    {"incremental":[{"id":"1","data":{"devices":{"edges":[{"node":{"id":"Q29ubmVjdG9yRGV2aWNlOjU=","name":"ConnectorDevice: Q29ubmVjdG9yRGV2aWNlOjU=","__typename":"ConnectorDevice"},"cursor":"ConnectorDeviceEdge: Q29ubmVjdG9yRGV2aWNlRWRnZToy"},{"node":{"id":"Q29ubmVjdG9yRGV2aWNlOjY=","name":"ConnectorDevice: Q29ubmVjdG9yRGV2aWNlOjY=","__typename":"ConnectorDevice"},"cursor":"ConnectorDeviceEdge: Q29ubmVjdG9yRGV2aWNlRWRnZToz"},{"node":{"id":"Q29ubmVjdG9yRGV2aWNlOjc=","name":"ConnectorDevice: Q29ubmVjdG9yRGV2aWNlOjc=","__typename":"ConnectorDevice"},"cursor":"ConnectorDeviceEdge: Q29ubmVjdG9yRGV2aWNlRWRnZTo0"}],"totalCount":123}}}],"completed":[{"id":"1"}]}
    {"incremental":[{"id":"0","data":{"system":"Connector: Q29ubmVjdG9yOjE=","description":"Connector: Q29ubmVjdG9yOjE="}}],"completed":[{"id":"0"}]}
    a: query($id:ID!){node(id: $id){__typename id ...on Connector{__typename id system version vendor}}} {"id":"Q29ubmVjdG9yOjE="}
    a: query($representations:[_Any!]!){_entities(representations: $representations){...on Connector{system description}}} {"representations":[{"__typename":"Connector","id":"Q29ubmVjdG9yOjE="}]}
    b: query($representations:[_Any!]!, $_v0:Int){_entities(representations: $representations){...on Connector{devices(first: $_v0){edges{node{__typename id name} cursor} totalCount}}}} {"_v0":10,"representations":[{"__typename":"Connector","id":"Q29ubmVjdG9yOjE="}]}
    "#);
}

const DASHBOARD_ONE_QUERY: &str = r#"
  query DashboardQuery($id: ID!) {
    node(id: $id) {
      __typename
      id
      ...DashboardFragment @defer(label: "testing")
    }
  }

  fragment DashboardFragment on Dashboard {
    id
    widget {
      __typename
      ... on SensorWidgetInfo {
        entity { feature { __typename ... on Sensor { value } } }
      }
      ... on ImageWidgetInfo {
        entity { feature { __typename ... on ImageFeature { file { url } } } }
      }
    }
  }
"#;

#[ntex::test]
async fn nested_defer_under_one_of_two_type_conditions() {
    let setup = start(
        "dashboard-one",
        json!({
            "a": {
                "query": {
                    "node": {
                        "__typename": "Dashboard",
                        "id": "RGFzaGJvYXJkOjE=",
                        "widget": {
                            "__typename": "ImageWidgetInfo",
                            "entity": { "feature": { "__typename": "ImageFeature", "file": { "url": "File: RmlsZTo0" } } }
                        }
                    }
                }
            }
        }),
    )
    .await;
    let parts = setup
        .run(
            DASHBOARD_ONE_QUERY,
            Some(json!({ "id": "RGFzaGJvYXJkOjE=" })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"node":{"__typename":"Dashboard","id":"RGFzaGJvYXJkOjE="}},"pending":[{"id":"0","path":["node"],"label":"testing"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"widget":{"__typename":"ImageWidgetInfo","entity":{"feature":{"__typename":"ImageFeature","file":{"url":"File: RmlsZTo0"}}}}}}],"completed":[{"id":"0"}]}
    a: query($id:ID!){node(id: $id){__typename id ...on Dashboard{id widget{__typename ...on SensorWidgetInfo{entity{feature{__typename ...on Sensor{value}}}} ...on ImageWidgetInfo{entity{feature{__typename ...on ImageFeature{file{url}}}}}}}}} {"id":"RGFzaGJvYXJkOjE="}
    "#);
}

#[ntex::test]
async fn nested_defer_under_both_type_conditions() {
    let setup = start(
        "dashboard-both",
        json!({
            "a": {
                "query": {
                    "node": {
                        "__typename": "Dashboard",
                        "id": "RGFzaGJvYXJkOjE=",
                        "widget": {
                            "__typename": "VideoWidgetInfo",
                            "entity": { "feature": { "__typename": "VideoFeature", "file": { "clip": { "duration": "VideoClip: VmlkZW9DbGlwOjU=" } } } }
                        }
                    }
                }
            }
        }),
    )
    .await;
    let parts = setup
        .run(
            r#"
              query DashboardQuery($id: ID!) {
                node(id: $id) {
                  __typename
                  id
                  ...DashboardFragment @defer(label: "testing")
                }
              }

              fragment DashboardFragment on Dashboard {
                id
                widget {
                  __typename
                  ... on ImageWidgetInfo {
                    entity { feature { __typename ... on ImageFeature { file { meta { url } } } } }
                  }
                  ... on VideoWidgetInfo {
                    entity { feature { __typename ... on VideoFeature { file { clip { duration } } } } }
                  }
                }
              }
            "#,
            Some(json!({ "id": "RGFzaGJvYXJkOjE=" })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"node":{"__typename":"Dashboard","id":"RGFzaGJvYXJkOjE="}},"pending":[{"id":"0","path":["node"],"label":"testing"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"widget":{"__typename":"VideoWidgetInfo","entity":{"feature":{"__typename":"VideoFeature","file":{"clip":{"duration":"VideoClip: VmlkZW9DbGlwOjU="}}}}}}}],"completed":[{"id":"0"}]}
    a: query($id:ID!){node(id: $id){__typename id ...on Dashboard{id widget{__typename ...on ImageWidgetInfo{entity{feature{__typename ...on ImageFeature{file{meta{url}}}}}} ...on VideoWidgetInfo{entity{feature{__typename ...on VideoFeature{file{clip{duration}}}}}}}}}} {"id":"RGFzaGJvYXJkOjE="}
    "#);
}

// ---- root `__typename` ----

fn suborga_with_names() -> Value {
    json!([
        { "__typename": "Organization", "id": "1" },
        { "__typename": "Organization", "id": "2", "name": "A" },
        { "__typename": "Organization", "id": "3" }
    ])
}

#[ntex::test]
/// Root `__typename` stays in the initial response when a root field is deferred.
async fn root_typename_stays_in_initial_part() {
    let setup = start_full(ORGA, mocks(suborga_mocks(suborga_with_names())), "").await;
    let deferred = setup.check("query { __typename currentUser { activeOrganization { id  suborga { id ...@defer { name } } } } }",
        None,
    )
    .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"__typename":"Query","currentUser":{"activeOrganization":{"id":"0","suborga":[{"id":"1"},{"id":"2"},{"id":"3"}]}}},"pending":[{"id":"0","path":["currentUser","activeOrganization","suborga",0]},{"id":"1","path":["currentUser","activeOrganization","suborga",1]},{"id":"2","path":["currentUser","activeOrganization","suborga",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":null}},{"id":"1","data":{"name":"A"}},{"id":"2","data":{"name":null}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    orga: query($representations:[_Any!]!){_entities(representations: $representations){...on Organization{name}}} {"representations":[{"__typename":"Organization","id":"1"},{"__typename":"Organization","id":"2"},{"__typename":"Organization","id":"3"}]}
    orga: query($representations:[_Any!]!){_entities(representations: $representations){...on Organization{suborga{__typename id}}}} {"representations":[{"__typename":"Organization","id":"0"}]}
    user: {__typename currentUser{activeOrganization{__typename id}}}
    "#);
}

#[ntex::test]
/// The initial part is still sent when its only field is root `__typename`.
async fn typename_only_initial_part_is_sent() {
    let setup = start_full(ORGA, mocks(suborga_mocks(suborga_with_names())), "").await;
    let deferred = setup
        .check(
            r#"
        query {
            ...OnlyTypename
            ... @defer {
                currentUser {
                    activeOrganization {
                        id
                        suborga {
                            id
                            name
                        }
                    }
                }
            }
        }

        fragment OnlyTypename on Query {
          __typename
        }
        "#,
            None,
        )
        .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"__typename":"Query"},"pending":[{"id":"0","path":[]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"currentUser":{"activeOrganization":{"id":"0","suborga":[{"id":"1","name":null},{"id":"2","name":"A"},{"id":"3","name":null}]}}}}],"completed":[{"id":"0"}]}
    orga: query($representations:[_Any!]!){_entities(representations: $representations){...on Organization{suborga{id name}}}} {"representations":[{"__typename":"Organization","id":"0"}]}
    user: {__typename}
    user: {currentUser{activeOrganization{__typename id}}}
    "#);
}

#[ntex::test]
/// Root `__typename` inside a deferred fragment is delivered in that patch.
async fn deferred_fragment_can_select_root_typename() {
    let setup = start_full(ORGA, mocks(suborga_mocks(suborga_with_names())), "").await;
    let deferred = setup.check("query { ...@defer { __typename currentUser { activeOrganization { id  suborga { id name } } } } }",
        None,
    )
    .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{},"pending":[{"id":"0","path":[]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"__typename":"Query","currentUser":{"activeOrganization":{"id":"0","suborga":[{"id":"1","name":null},{"id":"2","name":"A"},{"id":"3","name":null}]}}}}],"completed":[{"id":"0"}]}
    orga: query($representations:[_Any!]!){_entities(representations: $representations){...on Organization{suborga{id name}}}} {"representations":[{"__typename":"Organization","id":"0"}]}
    user: {__typename currentUser{activeOrganization{__typename id}}}
    "#);
}
