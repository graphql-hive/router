//! Which subgraph calls run, and with what: nothing runs twice, a deferred key needs no extra
//! call, variables reach the entity calls that need them, a mutation runs once.

use insta::assert_snapshot;
use serde_json::{json, Value};

use super::*;

#[ntex::test]
/// Each subgraph is called once, every deferred level runs only its own fetches.
async fn each_subgraph_is_called_once() {
    let setup = demo().await;
    let run = setup
        .check(
            r#"
            query {
                topProducts(first: 2) {
                    upc
                    name
                    ... @defer(label: "reviews") {
                        reviews {
                            body
                            author {
                                id
                                ... @defer(label: "author") { name }
                            }
                        }
                    }
                }
            }
            "#,
            None,
        )
        .await;
    assert_snapshot!(run.text, @r#"
    {"data":{"topProducts":[{"upc":"1","name":"Table"},{"upc":"2","name":"Couch"}]},"pending":[{"id":"0","path":["topProducts",0],"label":"reviews"},{"id":"1","path":["topProducts",1],"label":"reviews"}],"hasNext":true}
    {"pending":[{"id":"2","path":["topProducts",0,"reviews",0,"author"],"label":"author"},{"id":"3","path":["topProducts",0,"reviews",1,"author"],"label":"author"},{"id":"4","path":["topProducts",0,"reviews",2,"author"],"label":"author"},{"id":"5","path":["topProducts",0,"reviews",3,"author"],"label":"author"},{"id":"6","path":["topProducts",1,"reviews",0,"author"],"label":"author"},{"id":"7","path":["topProducts",1,"reviews",1,"author"],"label":"author"},{"id":"8","path":["topProducts",1,"reviews",2,"author"],"label":"author"},{"id":"9","path":["topProducts",1,"reviews",3,"author"],"label":"author"}],"incremental":[{"id":"0","data":{"reviews":[{"body":"Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.","author":{"id":"1"}},{"body":"Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi","author":{"id":"1"}},{"body":"sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.","author":{"id":"1"}},{"body":"Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem","author":{"id":"1"}}]}},{"id":"1","data":{"reviews":[{"body":"Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.","author":{"id":"1"}},{"body":"Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi","author":{"id":"1"}},{"body":"sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.","author":{"id":"1"}},{"body":"Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem","author":{"id":"1"}}]}}],"completed":[{"id":"0"},{"id":"1"}]}
    {"incremental":[{"id":"2","data":{"name":"Uri Goldshtein"}},{"id":"3","data":{"name":"Uri Goldshtein"}},{"id":"4","data":{"name":"Uri Goldshtein"}},{"id":"5","data":{"name":"Uri Goldshtein"}},{"id":"6","data":{"name":"Uri Goldshtein"}},{"id":"7","data":{"name":"Uri Goldshtein"}},{"id":"8","data":{"name":"Uri Goldshtein"}},{"id":"9","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"2"},{"id":"3"},{"id":"4"},{"id":"5"},{"id":"6"},{"id":"7"},{"id":"8"},{"id":"9"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    products: query($_v0:Int){topProducts(first: $_v0){__typename upc name}} {"_v0":2}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reviews{body author{__typename id}}}}} {"representations":[{"__typename":"Product","upc":"1"},{"__typename":"Product","upc":"2"}]}
    "#);
    for subgraph in ["products", "reviews", "accounts"] {
        let calls = run
            .text
            .lines()
            .filter(|line| line.starts_with(&format!("{subgraph}: ")))
            .count();
        assert_eq!(calls, 1, "{subgraph} called {calls} times");
    }
}

fn users_key_only_mocks() -> Value {
    json!({
        "accounts": {
            "query": {
                "users": [
                    { "__typename": "User", "id": "1" },
                    { "__typename": "User", "id": "2" },
                    { "__typename": "User", "id": "3" }
                ]
            }
        }
    })
}

/// The deferred key is already there, so one `users` call and nothing else.
#[ntex::test]
async fn key_only_defer_needs_no_extra_call() {
    let setup = start(users_key_only_mocks()).await;
    let parts = setup
        .run(
            "query GetUsers { users { ... @defer { id } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{},{},{}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"id":"1"}},{"id":"1","data":{"id":"2"}},{"id":"2","data":{"id":"3"}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: {users{__typename id}}
    "#);
}

#[ntex::test]
async fn key_only_defer_keeps_a_typename_sibling() {
    let setup = start(users_key_only_mocks()).await;
    let parts = setup
        .run(
            "query GetUsers { users { __typename ... @defer { id } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{"__typename":"User"},{"__typename":"User"},{"__typename":"User"}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"id":"1"}},{"id":"1","data":{"id":"2"}},{"id":"2","data":{"id":"3"}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: {users{__typename id}}
    "#);
}

#[ntex::test]
async fn key_only_defer_keeps_typename_inside_the_fragment() {
    let setup = start(users_key_only_mocks()).await;
    let parts = setup
        .run(
            "query GetUsers { users { ... @defer { __typename id } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{},{},{}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"__typename":"User","id":"1"}},{"id":"1","data":{"__typename":"User","id":"2"}},{"id":"2","data":{"__typename":"User","id":"3"}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: {users{__typename id}}
    "#);
}

/// Three of a user's products, starting at the 1-based `from`.
fn products(from: usize) -> Value {
    (from..from + 3)
        .map(|id| json!({ "__typename": "Product", "id": id.to_string() }))
        .collect()
}

fn reviews(from: usize) -> Value {
    (from..from + 3)
        .map(|id| json!({ "id": id.to_string() }))
        .collect()
}

// a user's products come from `products`, a product's reviews from `reviews`
fn user_products_mocks() -> Value {
    json!({
        "accounts": {
            "query": {
                "user": { "__typename": "User", "id": "1" },
                "users": [
                    { "__typename": "User", "id": "1" },
                    { "__typename": "User", "id": "2" },
                    { "__typename": "User", "id": "3" }
                ]
            }
        },
        "products": {
            "entities": [
                { "__typename": "User", "id": "1", "products(first: 5)": products(1), "products(first: 3)": products(1) },
                { "__typename": "User", "id": "2", "products(first: 5)": products(4) },
                { "__typename": "User", "id": "3", "products(first: 5)": products(7) }
            ]
        },
        "reviews": {
            "entities": [
                { "__typename": "Product", "id": "1", "reviews(first: 2)": reviews(1) },
                { "__typename": "Product", "id": "2", "reviews(first: 2)": reviews(4) },
                { "__typename": "Product", "id": "3", "reviews(first: 2)": reviews(7) }
            ]
        }
    })
}

/// The `products` call carries both the key and `$limit`.
#[ntex::test]
async fn forwarded_variable_goes_into_the_entity_call() {
    let setup = start(user_products_mocks()).await;
    let parts = setup
        .run(
            r#"
              query GetUserProducts($limit: Int!) {
                user(id: "1") {
                  id
                  ... @defer(label: "products") { products(first: $limit) { id } }
                }
              }
            "#,
            Some(json!({ "limit": 5 })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"user":{"id":"1"}},"pending":[{"id":"0","path":["user"],"label":"products"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"products":[{"id":"1"},{"id":"2"},{"id":"3"}]}}],"completed":[{"id":"0"}]}
    accounts: query($_v0:ID!){user(id: $_v0){__typename id}} {"_v0":"1"}
    products: query($representations:[_Any!]!, $limit:Int!){_entities(representations: $representations){...on User{products(first: $limit){id}}}} {"limit":5,"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn forwarded_variable_at_each_nesting_level() {
    let setup = start(user_products_mocks()).await;
    let parts = setup
        .run(
            r#"
              query GetUserProductsAndReviews($productLimit: Int!, $reviewLimit: Int!) {
                user(id: "1") {
                  id
                  ... @defer(label: "products") {
                    products(first: $productLimit) {
                      id
                      ... @defer(label: "reviews") { reviews(first: $reviewLimit) { id } }
                    }
                  }
                }
              }
            "#,
            Some(json!({ "productLimit": 3, "reviewLimit": 2 })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"user":{"id":"1"}},"pending":[{"id":"0","path":["user"],"label":"products"}],"hasNext":true}
    {"pending":[{"id":"1","path":["user","products",0],"label":"reviews"},{"id":"2","path":["user","products",1],"label":"reviews"},{"id":"3","path":["user","products",2],"label":"reviews"}],"incremental":[{"id":"0","data":{"products":[{"id":"1"},{"id":"2"},{"id":"3"}]}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"reviews":[{"id":"1"},{"id":"2"},{"id":"3"}]}},{"id":"2","data":{"reviews":[{"id":"4"},{"id":"5"},{"id":"6"}]}},{"id":"3","data":{"reviews":[{"id":"7"},{"id":"8"},{"id":"9"}]}}],"completed":[{"id":"1"},{"id":"2"},{"id":"3"}]}
    accounts: query($_v0:ID!){user(id: $_v0){__typename id}} {"_v0":"1"}
    products: query($representations:[_Any!]!, $productLimit:Int!){_entities(representations: $representations){...on User{products(first: $productLimit){__typename id}}}} {"productLimit":3,"representations":[{"__typename":"User","id":"1"}]}
    reviews: query($representations:[_Any!]!, $reviewLimit:Int!){_entities(representations: $representations){...on Product{reviews(first: $reviewLimit){id}}}} {"representations":[{"__typename":"Product","id":"1"},{"__typename":"Product","id":"2"},{"__typename":"Product","id":"3"}],"reviewLimit":2}
    "#);
}

#[ntex::test]
async fn forwarded_variable_over_a_list() {
    let setup = start(user_products_mocks()).await;
    let parts = setup
        .run(
            r#"
              query GetUsersProducts($limit: Int!) {
                users {
                  id
                  ... @defer(label: "products") { products(first: $limit) { id } }
                }
              }
            "#,
            Some(json!({ "limit": 5 })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{"id":"1"},{"id":"2"},{"id":"3"}]},"pending":[{"id":"0","path":["users",0],"label":"products"},{"id":"1","path":["users",1],"label":"products"},{"id":"2","path":["users",2],"label":"products"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"products":[{"id":"1"},{"id":"2"},{"id":"3"}]}},{"id":"1","data":{"products":[{"id":"4"},{"id":"5"},{"id":"6"}]}},{"id":"2","data":{"products":[{"id":"7"},{"id":"8"},{"id":"9"}]}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: {users{__typename id}}
    products: query($representations:[_Any!]!, $limit:Int!){_entities(representations: $representations){...on User{products(first: $limit){id}}}} {"limit":5,"representations":[{"__typename":"User","id":"1"},{"__typename":"User","id":"2"},{"__typename":"User","id":"3"}]}
    "#);
}

/// The mutation runs exactly once.
#[ntex::test]
async fn mutation_runs_once() {
    let setup = start(json!({
        "products": {
            "mutation": { "createProduct": { "__typename": "Product", "id": "1", "name": "Widget" } }
        },
        "inventory": {
            "entities": [{ "__typename": "Product", "id": "1", "price": 9.5 }]
        }
    }))
    .await;
    let parts = setup
        .run(
            r#"
              mutation {
                createProduct(name: "Widget") {
                  name
                  ... @defer(label: "pricing") { price }
                }
              }
            "#,
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"createProduct":{"name":"Widget"}},"pending":[{"id":"0","path":["createProduct"],"label":"pricing"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"price":9.5}}],"completed":[{"id":"0"}]}
    inventory: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price}}} {"representations":[{"__typename":"Product","id":"1"}]}
    products: mutation($_v0:String!){createProduct(name: $_v0){__typename name id}} {"_v0":"Widget"}
    "#);
}

fn users_with_review_counts_mocks() -> Value {
    let user = |id: &str, name: &str, birthday: &str| json!({ "__typename": "User", "id": id, "name": name, "birthday": birthday });
    let users = json!([
        user("1", "Ada", "1815-12-10"),
        user("2", "Grace", "1906-12-09"),
        user("3", "Linus", "1969-12-28")
    ]);
    json!({
        "accounts": {
            "query": { "users": users },
            "entities": users
        },
        "reviews": {
            "entities": [
                { "__typename": "User", "id": "1", "reviewCount": 3 },
                { "__typename": "User", "id": "2", "reviewCount": 1 },
                { "__typename": "User", "id": "3", "reviewCount": 0 }
            ]
        }
    })
}

/// `accounts` has both `users` and `user(id:)`. Only one `users` call.
#[ntex::test]
async fn list_anchor_uses_a_keyed_lookup() {
    let setup = start(users_with_review_counts_mocks()).await;
    let parts = setup
        .run(
            "{ users { ... @defer { id name } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{},{},{}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"id":"1","name":"Ada"}},{"id":"1","data":{"id":"2","name":"Grace"}},{"id":"2","data":{"id":"3","name":"Linus"}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{id name}}} {"representations":[{"__typename":"User","id":"1"},{"__typename":"User","id":"2"},{"__typename":"User","id":"3"}]}
    accounts: {users{__typename id}}
    "#);
}

#[ntex::test]
async fn same_and_other_subgraph_fields_are_both_kept() {
    let setup = start(users_with_review_counts_mocks()).await;
    let parts = setup
        .run(
            "{ users { name ... @defer { birthday reviewCount } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{"name":"Ada"},{"name":"Grace"},{"name":"Linus"}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"birthday":"1815-12-10","reviewCount":3}},{"id":"1","data":{"birthday":"1906-12-09","reviewCount":1}},{"id":"2","data":{"birthday":"1969-12-28","reviewCount":0}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{birthday}}} {"representations":[{"__typename":"User","id":"1"},{"__typename":"User","id":"2"},{"__typename":"User","id":"3"}]}
    accounts: {users{__typename name id}}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviewCount}}} {"representations":[{"__typename":"User","id":"1"},{"__typename":"User","id":"2"},{"__typename":"User","id":"3"}]}
    "#);
}

#[ntex::test]
/// Both `reviews` calls are in one deferred part, so they go out as one batched request.
async fn entity_calls_of_one_deferred_part_are_batched() {
    let parts = demo()
        .await
        .run(
            "{ topProducts(first: 1) { upc } me { id } ... @defer { topProducts(first: 1) { reviews { id } } me { reviews { id } } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"topProducts":[{"upc":"1"}],"me":{"id":"1"}},"pending":[{"id":"0","path":[]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"topProducts":[{"reviews":[{"id":"1"},{"id":"2"},{"id":"3"},{"id":"4"}]}],"me":{"reviews":[{"id":"1"},{"id":"2"}]}}}],"completed":[{"id":"0"}]}
    accounts: {me{__typename id}}
    accounts: {me{id}}
    products: query($_v0:Int){topProducts(first: $_v0){__typename upc}} {"_v0":1}
    products: query($_v0:Int){topProducts(first: $_v0){upc}} {"_v0":1}
    reviews: query($__batch_reps_0:[_Any!]!, $__batch_reps_1:[_Any!]!){_e0: _entities(representations: $__batch_reps_0){...on User{reviews{...a}}} _e1: _entities(representations: $__batch_reps_1){...on Product{reviews{...a}}}}

    fragment a on Review {id}
     {"__batch_reps_0":[{"__typename":"User","id":"1"}],"__batch_reps_1":[{"__typename":"Product","upc":"1"}]}
    "#);
}

#[ntex::test]
/// A deferred fragment goes out as soon as its own data is in, a slow sibling doesn't hold it.
async fn a_fast_defer_does_not_wait_for_a_slow_sibling() {
    let setup = start_on(
        "supergraph.graphql",
        TestSubgraphs::builder()
            .with_path_delay("/inventory", std::time::Duration::from_millis(500)),
        "",
    )
    .await;
    let run = setup
        .check(
            r#"{ topProducts(first: 1) { upc ... @defer(label: "slow") { inStock } ... @defer(label: "fast") { name } } }"#,
            None,
        )
        .await;
    // the snapshot sorts parts, so check the real order here
    assert_eq!(
        run.parts[1]["incremental"][0]["data"],
        json!({ "name": "Table" }),
        "the fast part goes out first: {:?}",
        run.parts
    );
    assert_snapshot!(run.text, @r#"
    {"data":{"topProducts":[{"upc":"1"}]},"pending":[{"id":"0","path":["topProducts",0],"label":"slow"},{"id":"1","path":["topProducts",0],"label":"fast"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"inStock":true}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"name":"Table"}}],"completed":[{"id":"1"}]}
    inventory: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{inStock}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    products: query($_v0:Int){topProducts(first: $_v0){__typename upc}} {"_v0":1}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    "#);
}

#[ntex::test]
/// A fetch of a deferred fragment waits for the fetches it reads from, not for a slow sibling's.
async fn a_nested_defer_does_not_wait_for_a_slow_sibling_of_its_parent() {
    let setup = start_on(
        "supergraph.graphql",
        TestSubgraphs::builder()
            .with_path_delay("/inventory", std::time::Duration::from_millis(500)),
        "",
    )
    .await;
    let run = setup
        .check(
            r#"{ topProducts(first: 1) {
                upc
                ... @defer(label: "slow") { inStock }
                ... @defer(label: "fast") { reviews { id ... @defer(label: "author") { author { name } } } }
            } }"#,
            None,
        )
        .await;
    // the snapshot sorts parts, so check the real order here: labels in the order they completed
    let label_of = |id: &Value| {
        run.parts
            .iter()
            .flat_map(|part| part["pending"].as_array().cloned().unwrap_or_default())
            .find(|item| &item["id"] == id)
            .and_then(|item| item["label"].as_str().map(str::to_string))
            .unwrap_or_default()
    };
    let order: Vec<String> = run.parts[1..]
        .iter()
        .flat_map(|part| part["completed"].as_array().cloned().unwrap_or_default())
        .map(|item| label_of(&item["id"]))
        .collect();
    let last_author = order.iter().rposition(|label| label == "author").unwrap();
    let slow = order.iter().position(|label| label == "slow").unwrap();
    assert!(
        last_author < slow,
        "the nested part goes out before the slow sibling: {order:?}"
    );
    assert_snapshot!(run.text, @r#"
    {"data":{"topProducts":[{"upc":"1"}]},"pending":[{"id":"0","path":["topProducts",0],"label":"slow"},{"id":"1","path":["topProducts",0],"label":"fast"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"inStock":true}}],"completed":[{"id":"0"}]}
    {"pending":[{"id":"2","path":["topProducts",0,"reviews",0],"label":"author"},{"id":"3","path":["topProducts",0,"reviews",1],"label":"author"},{"id":"4","path":["topProducts",0,"reviews",2],"label":"author"},{"id":"5","path":["topProducts",0,"reviews",3],"label":"author"}],"incremental":[{"id":"1","data":{"reviews":[{"id":"1"},{"id":"2"},{"id":"3"},{"id":"4"}]}}],"completed":[{"id":"1"}]}
    {"incremental":[{"id":"2","data":{"author":{"name":"Uri Goldshtein"}}},{"id":"3","data":{"author":{"name":"Uri Goldshtein"}}},{"id":"4","data":{"author":{"name":"Uri Goldshtein"}}},{"id":"5","data":{"author":{"name":"Uri Goldshtein"}}}],"completed":[{"id":"2"},{"id":"3"},{"id":"4"},{"id":"5"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    inventory: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{inStock}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    products: query($_v0:Int){topProducts(first: $_v0){__typename upc}} {"_v0":1}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reviews{__typename id}}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on Review{author{__typename id}}}} {"representations":[{"__typename":"Review","id":"1"},{"__typename":"Review","id":"2"},{"__typename":"Review","id":"3"},{"__typename":"Review","id":"4"}]}
    "#);
}

/// A client that goes away stops the deferred work: the fetch that waits for the slow
/// `reviews` call never goes out.
#[ntex::test]
async fn disconnect_stops_deferred_fetches() {
    use futures::StreamExt;
    use std::time::Duration;

    let Setup { subgraphs, router } = start_on(
        "supergraph.graphql",
        TestSubgraphs::builder().with_path_delay("/reviews", Duration::from_millis(1000)),
        "",
    )
    .await;
    // `reviews` is slow, `products` waits for it
    let query = "{ me { id ... @defer { reviews { body product { name } } } } }";
    let accept = some_header_map!(http::header::ACCEPT => "multipart/mixed");
    let products_calls = || {
        subgraphs
            .get_requests_log("products")
            .map_or(0, |log| log.len())
    };

    let mut response = router
        .send_graphql_request(query, None, accept.clone())
        .await;
    let primary = response.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&primary).contains(r#""hasNext":true"#));
    // the deferred `reviews` call can still be on its way, leave once it's in
    let reviews_calls = || {
        subgraphs
            .get_requests_log("reviews")
            .map_or(0, |log| log.len())
    };
    for _ in 0..50 {
        if reviews_calls() > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(reviews_calls(), 1);
    drop(response);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        products_calls(),
        0,
        "a deferred fetch ran after the client left"
    );

    // the same request read to the end does call `products`
    let mut response = router.send_graphql_request(query, None, accept).await;
    while response.next().await.is_some() {}
    assert_eq!(products_calls(), 1);
}

const ENTITY_REUSE: &str =
    "traffic_shaping:\n  entity_reuse: true\n  all:\n    dedupe_enabled: false\n";
const DEFERRED_DEEP_PRODUCTS: &str =
    "{ topProducts { upc name ... @defer { reviews { author { reviews { product { upc name } } } } } } }";

fn products_requests(run: &Run) -> usize {
    run.text
        .lines()
        .filter(|line| line.starts_with("products: "))
        .count()
}

/// With entity reuse on, the deferred deep products are copied from `topProducts` in the
/// primary part, so `products` is called once.
#[ntex::test]
async fn deferred_fetch_copies_entities_of_the_primary_part() {
    let setup = start_full("supergraph.graphql", None, ENTITY_REUSE).await;
    let run = setup.check(DEFERRED_DEEP_PRODUCTS, None).await;
    assert_eq!(products_requests(&run), 1, "{}", run.text);
}

/// The primary part goes out with the error for `topProducts`, so the error is no longer in the
/// request's context. Copying still stays off: the deferred deep products are asked for, not
/// copied as a `null` without an error.
#[ntex::test]
async fn deferred_fetch_does_not_copy_after_an_error_went_out() {
    let on_request: OnRequest = Box::new(|req: RequestLike| {
        let body = String::from_utf8_lossy(req.body.as_deref()?).to_string();
        if req.path != "/products" || !body.contains("topProducts") {
            return None;
        }
        respond(json!({
            "data": { "topProducts": [
                { "__typename": "Product", "upc": "1", "name": null },
                { "__typename": "Product", "upc": "2", "name": "Couch" }
            ] },
            "errors": [{ "message": "name lookup failed", "path": ["topProducts", 0, "name"] }]
        }))
    });
    let setup = start_full("supergraph.graphql", Some(on_request), ENTITY_REUSE).await;
    let run = setup.send(DEFERRED_DEEP_PRODUCTS, None).await;
    assert_eq!(products_requests(&run), 2, "{}", run.text);
    assert!(
        !run.text.contains(r#"{"upc":"1","name":null}}"#),
        "{}",
        run.text
    );
}
