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
    let setup = start(user_reviews_mocks()).await;
    assert_snapshot!(setup.run(USER_REVIEWS_QUERY, None, Plain::Stripped).await, @r#"
    {"data":{"user":{"name":"Ada"}},"pending":[{"id":"0","path":["user"],"label":"reviews"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"reviews":[{"title":"Great","body":"Works well"},{"title":"Okay","body":"Does the job"},{"title":"Bad","body":"Broke in a week"}]}}],"completed":[{"id":"0"}]}
    accounts: query($_v0:ID!){user(id: $_v0){__typename name id}} {"_v0":"1"}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{title body}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

// `name` comes with the `user` call, the deferred `email` and `address` from entity calls back
// to `accounts`.

fn user_contact_mocks() -> Value {
    let user = json!({ "__typename": "User", "id": "1", "name": "Ada", "email": "ada@example.com", "address": "1 Main St" });
    json!({
        "accounts": {
            "query": { "user": user },
            "entities": [user]
        }
    })
}

#[ntex::test]
async fn nested_defers_go_out_in_order() {
    let setup = start(user_contact_mocks()).await;
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
    {"data":{"user":{"name":"Ada"}},"pending":[{"id":"0","path":["user"],"label":"outer"}],"hasNext":true}
    {"pending":[{"id":"1","path":["user"],"label":"inner"}],"incremental":[{"id":"0","data":{"email":"ada@example.com"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"address":"1 Main St"}}],"completed":[{"id":"1"}]}
    accounts: query($_v0:ID!){user(id: $_v0){__typename name id}} {"_v0":"1"}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{address}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{email}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn unlabeled_inner_defer() {
    let setup = start(user_contact_mocks()).await;
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
    {"data":{"user":{"name":"Ada"}},"pending":[{"id":"0","path":["user"],"label":"outer"}],"hasNext":true}
    {"pending":[{"id":"1","path":["user"]}],"incremental":[{"id":"0","data":{"email":"ada@example.com"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"address":"1 Main St"}}],"completed":[{"id":"1"}]}
    accounts: query($_v0:ID!){user(id: $_v0){__typename name id}} {"_v0":"1"}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{address}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{email}}} {"representations":[{"__typename":"User","id":"1"}]}
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
    let setup = start(user_contact_mocks()).await;
    assert_snapshot!(setup.run(TWO_SIBLINGS_QUERY, None, Plain::Stripped).await, @r#"
    {"data":{"user":{"name":"Ada"}},"pending":[{"id":"0","path":["user"],"label":"contact"},{"id":"1","path":["user"],"label":"location"}],"hasNext":true}
    {"incremental":[{"id":"1","data":{"email":"ada@example.com","address":"1 Main St"}}],"completed":[{"id":"1"}]}
    {"incremental":[{"id":"0","data":{"email":"ada@example.com"}}],"completed":[{"id":"0"}]}
    accounts: query($_v0:ID!){user(id: $_v0){__typename name id}} {"_v0":"1"}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{email address}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{email}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
/// The only field under `me` is deferred.
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
/// A deferred fragment under a list.
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
        { "__typename": "User", "id": "1", "name": "Ada" },
        { "__typename": "User", "id": "2", "name": "Grace" },
        { "__typename": "User", "id": "3", "name": "Linus" }
    ]);
    let reviews = json!([
        { "__typename": "Review", "id": "1", "rating": 5 },
        { "__typename": "Review", "id": "2", "rating": 4 },
        { "__typename": "Review", "id": "3", "rating": 3 }
    ]);
    json!({
        "products": {
            "query": {
                "products": [
                    { "__typename": "Product", "id": "1", "name": "Table" },
                    { "__typename": "Product", "id": "2", "name": "Chair" },
                    { "__typename": "Product", "id": "3", "name": "Couch" }
                ]
            }
        },
        "accounts": {
            "query": { "users": users },
            "entities": users
        },
        "reviews": {
            "query": { "topReviews": reviews },
            "entities": reviews
        }
    })
}

#[ntex::test]
async fn defer_under_the_second_root_field() {
    let setup = start(roots_mocks()).await;
    let parts = setup
        .run(
            "query t { products { id name } users { __typename ... @defer { id name } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"products":[{"id":"1","name":"Table"},{"id":"2","name":"Chair"},{"id":"3","name":"Couch"}],"users":[{"__typename":"User"},{"__typename":"User"},{"__typename":"User"}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"id":"1","name":"Ada"}},{"id":"1","data":{"id":"2","name":"Grace"}},{"id":"2","data":{"id":"3","name":"Linus"}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{id name}}} {"representations":[{"__typename":"User","id":"1"},{"__typename":"User","id":"2"},{"__typename":"User","id":"3"}]}
    accounts: {users{__typename id}}
    products: {products{id name}}
    "#);
}

#[ntex::test]
async fn defer_under_the_third_root_field() {
    let setup = start(roots_mocks()).await;
    let parts = setup
        .run(
            "query t { products { id name } users { id name } topReviews { __typename ... @defer { id rating } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"products":[{"id":"1","name":"Table"},{"id":"2","name":"Chair"},{"id":"3","name":"Couch"}],"users":[{"id":"1","name":"Ada"},{"id":"2","name":"Grace"},{"id":"3","name":"Linus"}],"topReviews":[{"__typename":"Review"},{"__typename":"Review"},{"__typename":"Review"}]},"pending":[{"id":"0","path":["topReviews",0]},{"id":"1","path":["topReviews",1]},{"id":"2","path":["topReviews",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"id":"1","rating":5}},{"id":"1","data":{"id":"2","rating":4}},{"id":"2","data":{"id":"3","rating":3}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: {users{id name}}
    products: {products{id name}}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on Review{id rating}}} {"representations":[{"__typename":"Review","id":"1"},{"__typename":"Review","id":"2"},{"__typename":"Review","id":"3"}]}
    reviews: {topReviews{__typename id}}
    "#);
}

#[ntex::test]
async fn path_crosses_intermediate_lists() {
    let product = |id: &str, name: &str, height: f64| json!({ "__typename": "Product", "id": id, "name": name, "dimensions": { "height": height } });
    let review = |id: &str, product: &str| json!({ "__typename": "Review", "id": id, "product": { "__typename": "Product", "id": product } });
    let setup = start(json!({
        "accounts": {
            "query": {
                "users": [
                    { "__typename": "User", "id": "1" },
                    { "__typename": "User", "id": "2" }
                ]
            }
        },
        "reviews": {
            "entities": [
                { "__typename": "User", "id": "1", "reviews": [review("1", "1"), review("2", "2")] },
                { "__typename": "User", "id": "2", "reviews": [review("3", "3")] }
            ]
        },
        "products": {
            "entities": [product("1", "Table", 30.0), product("2", "Chair", 40.0), product("3", "Couch", 50.0)]
        },
        "inventory": {
            "entities": [product("1", "Table", 30.0), product("2", "Chair", 40.0), product("3", "Couch", 50.0)]
        }
    }))
    .await;
    let parts = setup
        .run(
            r#"
              {
                users {
                  reviews {
                    product {
                      name
                      ... @defer(label: "dimensions") { dimensions { height } }
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
    {"data":{"users":[{"reviews":[{"product":{"name":"Table"}},{"product":{"name":"Chair"}}]},{"reviews":[{"product":{"name":"Couch"}}]}]},"pending":[{"id":"0","path":["users",0,"reviews",0,"product"],"label":"dimensions"},{"id":"1","path":["users",0,"reviews",1,"product"],"label":"dimensions"},{"id":"2","path":["users",1,"reviews",0,"product"],"label":"dimensions"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"dimensions":{"height":30.0}}},{"id":"1","data":{"dimensions":{"height":40.0}}},{"id":"2","data":{"dimensions":{"height":50.0}}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: {users{__typename id}}
    inventory: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{dimensions{height}}}} {"representations":[{"__typename":"Product","id":"1"},{"__typename":"Product","id":"2"},{"__typename":"Product","id":"3"}]}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","id":"1"},{"__typename":"Product","id":"2"},{"__typename":"Product","id":"3"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{product{__typename id}}}}} {"representations":[{"__typename":"User","id":"1"},{"__typename":"User","id":"2"}]}
    "#);
}

// ---- abstract types ----

#[ntex::test]
/// `... on Product @defer` on an object field, under a type condition on an interface.
async fn defer_on_an_object_under_an_interface() {
    let setup = start(json!({
        "products": {
            "query": { "node": {
                "__typename": "Product", "id": "1", "name": "Desk set",
                "bundle": [{ "quantity": 2, "product": { "__typename": "Product", "id": "2", "name": "Lamp" } }]
            } },
            "entities": [{ "__typename": "Product", "id": "2", "name": "Lamp" }]
        }
    }))
    .await;
    let deferred = setup
        .check(
            r#"query {
            node(id: "1") {
              ... on Product {
                id
                name
                bundle {
                  quantity
                  product {
                    ... on Product @defer {
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
    {"data":{"node":{"id":"1","name":"Desk set","bundle":[{"quantity":2,"product":{}}]}},"pending":[{"id":"0","path":["node","bundle",0,"product"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Lamp"}}],"completed":[{"id":"0"}]}
    products: query($_v0:ID!){node(id: $_v0){__typename ...on Product{id name bundle{quantity product{__typename id}}}}} {"_v0":"1"}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","id":"2"}]}
    "#);
}

#[ntex::test]
async fn defer_under_a_type_condition_on_an_abstract_parent() {
    let edge = |id: &str, name: &str| {
        json!({
            "cursor": format!("cursor-{id}"),
            "node": { "__typename": "Variant", "id": id, "name": name }
        })
    };
    let setup = start(json!({
        "products": {
            "query": {
                "node": {
                    "__typename": "Product",
                    "id": "1",
                    "name": "Desk",
                    "brand": "Acme",
                    "color": "Oak",
                    "description": "A sturdy desk"
                }
            },
            "entities": [{
                "__typename": "Product",
                "id": "1",
                "name": "Desk",
                "description": "A sturdy desk"
            }]
        },
        "inventory": {
            "entities": [{
                "__typename": "Product",
                "id": "1",
                "variants(first: 10)": {
                    "totalCount": 3,
                    "edges": [edge("11", "Small"), edge("12", "Medium"), edge("13", "Large")]
                }
            }]
        }
    }))
    .await;
    let parts = setup
        .run(
            r#"
              query ProductDetails($id: ID!) {
                node(id: $id) {
                  __typename
                  id
                  ... on Product @defer(label: "details") { name description }
                  ...ProductHeader
                  ...ProductVariants @defer(label: "variants")
                }
              }

              fragment ProductHeader on Product { name brand color }

              fragment ProductVariants on Product {
                id
                variants(first: 10) {
                  edges {
                    node { ... on Variant { id name __typename } }
                    cursor
                  }
                  totalCount
                }
              }
            "#,
            Some(json!({ "id": "1" })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"node":{"__typename":"Product","id":"1","name":"Desk","brand":"Acme","color":"Oak"}},"pending":[{"id":"0","path":["node"],"label":"details"},{"id":"1","path":["node"],"label":"variants"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Desk","description":"A sturdy desk"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"variants":{"edges":[{"node":{"id":"11","name":"Small","__typename":"Variant"},"cursor":"cursor-11"},{"node":{"id":"12","name":"Medium","__typename":"Variant"},"cursor":"cursor-12"},{"node":{"id":"13","name":"Large","__typename":"Variant"},"cursor":"cursor-13"}],"totalCount":3}}}],"completed":[{"id":"1"}]}
    inventory: query($representations:[_Any!]!, $_v0:Int){_entities(representations: $representations){...on Product{variants(first: $_v0){edges{node{__typename id name} cursor} totalCount}}}} {"_v0":10,"representations":[{"__typename":"Product","id":"1"}]}
    products: query($id:ID!){node(id: $id){__typename id ...on Product{__typename id name brand color}}} {"id":"1"}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name description}}} {"representations":[{"__typename":"Product","id":"1"}]}
    "#);
}

const COLLECTION_QUERY_ONE: &str = r#"
  query CollectionQuery($id: ID!) {
    node(id: $id) {
      __typename
      id
      ...CollectionFragment @defer(label: "hero")
    }
  }

  fragment CollectionFragment on Collection {
    id
    hero {
      __typename
      ... on TextHero {
        content { media { __typename ... on Text { value } } }
      }
      ... on ImageHero {
        content { media { __typename ... on Image { file { url } } } }
      }
    }
  }
"#;

#[ntex::test]
async fn nested_defer_under_one_of_two_type_conditions() {
    let setup = start(json!({
        "products": {
            "query": {
                "node": {
                    "__typename": "Collection",
                    "id": "1",
                    "hero": {
                        "__typename": "ImageHero",
                        "content": { "media": { "__typename": "Image", "file": { "url": "https://example.com/desk.png" } } }
                    }
                }
            }
        }
    }))
    .await;
    let parts = setup
        .run(
            COLLECTION_QUERY_ONE,
            Some(json!({ "id": "1" })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"node":{"__typename":"Collection","id":"1"}},"pending":[{"id":"0","path":["node"],"label":"hero"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"hero":{"__typename":"ImageHero","content":{"media":{"__typename":"Image","file":{"url":"https://example.com/desk.png"}}}}}}],"completed":[{"id":"0"}]}
    products: query($id:ID!){node(id: $id){__typename id ...on Collection{id hero{__typename ...on TextHero{content{media{__typename ...on Text{value}}}} ...on ImageHero{content{media{__typename ...on Image{file{url}}}}}}}}} {"id":"1"}
    "#);
}

#[ntex::test]
async fn nested_defer_under_both_type_conditions() {
    let setup = start(json!({
        "products": {
            "query": {
                "node": {
                    "__typename": "Collection",
                    "id": "1",
                    "hero": {
                        "__typename": "VideoHero",
                        "content": { "media": { "__typename": "Video", "file": { "clip": { "duration": "1:30" } } } }
                    }
                }
            }
        }
    }))
    .await;
    let parts = setup
        .run(
            r#"
              query CollectionQuery($id: ID!) {
                node(id: $id) {
                  __typename
                  id
                  ...CollectionFragment @defer(label: "hero")
                }
              }

              fragment CollectionFragment on Collection {
                id
                hero {
                  __typename
                  ... on ImageHero {
                    content { media { __typename ... on Image { file { meta { url } } } } }
                  }
                  ... on VideoHero {
                    content { media { __typename ... on Video { file { clip { duration } } } } }
                  }
                }
              }
            "#,
            Some(json!({ "id": "1" })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"node":{"__typename":"Collection","id":"1"}},"pending":[{"id":"0","path":["node"],"label":"hero"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"hero":{"__typename":"VideoHero","content":{"media":{"__typename":"Video","file":{"clip":{"duration":"1:30"}}}}}}}],"completed":[{"id":"0"}]}
    products: query($id:ID!){node(id: $id){__typename id ...on Collection{id hero{__typename ...on ImageHero{content{media{__typename ...on Image{file{meta{url}}}}}} ...on VideoHero{content{media{__typename ...on Video{file{clip{duration}}}}}}}}}} {"id":"1"}
    "#);
}

// ---- root `__typename` ----

fn related_with_names() -> Value {
    json!([
        { "__typename": "Product", "id": "1" },
        { "__typename": "Product", "id": "2", "name": "Chair" },
        { "__typename": "Product", "id": "3" }
    ])
}

#[ntex::test]
/// Root `__typename` stays in the initial response when a root field is deferred.
async fn root_typename_stays_in_initial_part() {
    let setup = start(related_products_mocks(related_with_names())).await;
    let deferred = setup.check("query { __typename me { favoriteProduct { id relatedProducts { id ...@defer { name } } } } }",
        None,
    )
    .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{"__typename":"Query","me":{"favoriteProduct":{"id":"0","relatedProducts":[{"id":"1"},{"id":"2"},{"id":"3"}]}}},"pending":[{"id":"0","path":["me","favoriteProduct","relatedProducts",0]},{"id":"1","path":["me","favoriteProduct","relatedProducts",1]},{"id":"2","path":["me","favoriteProduct","relatedProducts",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":null}},{"id":"1","data":{"name":"Chair"}},{"id":"2","data":{"name":null}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: {me{favoriteProduct{__typename id}}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","id":"1"},{"__typename":"Product","id":"2"},{"__typename":"Product","id":"3"}]}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{relatedProducts{__typename id}}}} {"representations":[{"__typename":"Product","id":"0"}]}
    "#);
}

#[ntex::test]
/// The initial part is still sent when its only field is root `__typename`.
async fn typename_only_initial_part_is_sent() {
    let setup = start(related_products_mocks(related_with_names())).await;
    let deferred = setup
        .check(
            r#"
        query {
            ...OnlyTypename
            ... @defer {
                me {
                    favoriteProduct {
                        id
                        relatedProducts {
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
    {"incremental":[{"id":"0","data":{"me":{"favoriteProduct":{"id":"0","relatedProducts":[{"id":"1","name":null},{"id":"2","name":"Chair"},{"id":"3","name":null}]}}}}],"completed":[{"id":"0"}]}
    accounts: {me{favoriteProduct{__typename id}}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{relatedProducts{id name}}}} {"representations":[{"__typename":"Product","id":"0"}]}
    "#);
}

#[ntex::test]
/// Root `__typename` inside a deferred fragment is delivered in that patch.
async fn deferred_fragment_can_select_root_typename() {
    let setup = start(related_products_mocks(related_with_names())).await;
    let deferred = setup.check("query { ...@defer { __typename me { favoriteProduct { id relatedProducts { id name } } } } }",
        None,
    )
    .await;
    assert_snapshot!(deferred.text, @r#"
    {"data":{},"pending":[{"id":"0","path":[]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"__typename":"Query","me":{"favoriteProduct":{"id":"0","relatedProducts":[{"id":"1","name":null},{"id":"2","name":"Chair"},{"id":"3","name":null}]}}}}],"completed":[{"id":"0"}]}
    accounts: {me{favoriteProduct{__typename id}}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{relatedProducts{id name}}}} {"representations":[{"__typename":"Product","id":"0"}]}
    "#);
}
