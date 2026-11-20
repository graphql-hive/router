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
        "a": {
            "query": {
                "users": [
                    { "__typename": "User", "id": "VXNlcjox" },
                    { "__typename": "User", "id": "VXNlcjoy" },
                    { "__typename": "User", "id": "VXNlcjoz" }
                ]
            }
        },
        "b": {}
    })
}

/// The deferred key is already there, so one request to `a`, nothing to `b`.
#[ntex::test]
async fn key_only_defer_needs_no_extra_call() {
    let setup = start("users-key-only", users_key_only_mocks()).await;
    let parts = setup
        .run(
            "query GetUsers { users { ... @defer { id } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{},{},{}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"id":"VXNlcjox"}},{"id":"1","data":{"id":"VXNlcjoy"}},{"id":"2","data":{"id":"VXNlcjoz"}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    a: {users{__typename id}}
    "#);
}

#[ntex::test]
async fn key_only_defer_keeps_a_typename_sibling() {
    let setup = start("users-key-only", users_key_only_mocks()).await;
    let parts = setup
        .run(
            "query GetUsers { users { __typename ... @defer { id } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{"__typename":"User"},{"__typename":"User"},{"__typename":"User"}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"id":"VXNlcjox"}},{"id":"1","data":{"id":"VXNlcjoy"}},{"id":"2","data":{"id":"VXNlcjoz"}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    a: {users{__typename id}}
    "#);
}

#[ntex::test]
async fn key_only_defer_keeps_typename_inside_the_fragment() {
    let setup = start("users-key-only", users_key_only_mocks()).await;
    let parts = setup
        .run(
            "query GetUsers { users { ... @defer { __typename id } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{},{},{}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"__typename":"User","id":"VXNlcjox"}},{"id":"1","data":{"__typename":"User","id":"VXNlcjoy"}},{"id":"2","data":{"__typename":"User","id":"VXNlcjoz"}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    a: {users{__typename id}}
    "#);
}

// base64 of `Post:1`..`Post:9` and `Comment:1`..`Comment:9`
const POST_IDS: [&str; 9] = [
    "UG9zdDox", "UG9zdDoy", "UG9zdDoz", "UG9zdDo0", "UG9zdDo1", "UG9zdDo2", "UG9zdDo3", "UG9zdDo4",
    "UG9zdDo5",
];
const COMMENT_IDS: [&str; 9] = [
    "Q29tbWVudDox",
    "Q29tbWVudDoy",
    "Q29tbWVudDoz",
    "Q29tbWVudDo0",
    "Q29tbWVudDo1",
    "Q29tbWVudDo2",
    "Q29tbWVudDo3",
    "Q29tbWVudDo4",
    "Q29tbWVudDo5",
];

/// Three posts, starting at the 1-based `from`.
fn posts(from: usize) -> Value {
    POST_IDS[from - 1..from + 2]
        .iter()
        .map(|id| json!({ "__typename": "Post", "id": id }))
        .collect()
}

fn comments(from: usize) -> Value {
    COMMENT_IDS[from - 1..from + 2]
        .iter()
        .map(|id| json!({ "id": id }))
        .collect()
}

fn posts_mocks() -> Value {
    json!({
        "a": {
            "query": {
                "user": { "__typename": "User", "id": "VXNlcjox" },
                "users": [
                    { "__typename": "User", "id": "VXNlcjox" },
                    { "__typename": "User", "id": "VXNlcjoy" },
                    { "__typename": "User", "id": "VXNlcjoz" }
                ]
            }
        },
        "b": {
            "entities": [
                { "__typename": "User", "id": "VXNlcjox", "posts(first: 5)": posts(1), "posts(first: 3)": posts(1) },
                { "__typename": "User", "id": "VXNlcjoy", "posts(first: 5)": posts(4) },
                { "__typename": "User", "id": "VXNlcjoz", "posts(first: 5)": posts(7) }
            ]
        },
        "c": {
            "entities": [
                { "__typename": "Post", "id": "UG9zdDox", "comments(first: 2)": comments(1) },
                { "__typename": "Post", "id": "UG9zdDoy", "comments(first: 2)": comments(4) },
                { "__typename": "Post", "id": "UG9zdDoz", "comments(first: 2)": comments(7) }
            ]
        }
    })
}

/// The `b` call carries both the key and `$limit`.
#[ntex::test]
async fn forwarded_variable_goes_into_the_entity_call() {
    let setup = start("posts", posts_mocks()).await;
    let parts = setup
        .run(
            r#"
              query GetUserPosts($limit: Int!) {
                user(id: "1") {
                  id
                  ... @defer(label: "posts") { posts(first: $limit) { id } }
                }
              }
            "#,
            Some(json!({ "limit": 5 })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"user":{"id":"VXNlcjox"}},"pending":[{"id":"0","path":["user"],"label":"posts"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"posts":[{"id":"UG9zdDox"},{"id":"UG9zdDoy"},{"id":"UG9zdDoz"}]}}],"completed":[{"id":"0"}]}
    a: query($_v0:ID!){user(id: $_v0){__typename id}} {"_v0":"1"}
    b: query($representations:[_Any!]!, $limit:Int!){_entities(representations: $representations){...on User{posts(first: $limit){id}}}} {"limit":5,"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    "#);
}

#[ntex::test]
async fn forwarded_variable_at_each_nesting_level() {
    let setup = start("posts", posts_mocks()).await;
    let parts = setup
        .run(
            r#"
              query GetUserPostsAndComments($postLimit: Int!, $commentLimit: Int!) {
                user(id: "1") {
                  id
                  ... @defer(label: "posts") {
                    posts(first: $postLimit) {
                      id
                      ... @defer(label: "comments") { comments(first: $commentLimit) { id } }
                    }
                  }
                }
              }
            "#,
            Some(json!({ "postLimit": 3, "commentLimit": 2 })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"user":{"id":"VXNlcjox"}},"pending":[{"id":"0","path":["user"],"label":"posts"}],"hasNext":true}
    {"pending":[{"id":"1","path":["user","posts",0],"label":"comments"},{"id":"2","path":["user","posts",1],"label":"comments"},{"id":"3","path":["user","posts",2],"label":"comments"}],"incremental":[{"id":"0","data":{"posts":[{"id":"UG9zdDox"},{"id":"UG9zdDoy"},{"id":"UG9zdDoz"}]}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"comments":[{"id":"Q29tbWVudDox"},{"id":"Q29tbWVudDoy"},{"id":"Q29tbWVudDoz"}]}},{"id":"2","data":{"comments":[{"id":"Q29tbWVudDo0"},{"id":"Q29tbWVudDo1"},{"id":"Q29tbWVudDo2"}]}},{"id":"3","data":{"comments":[{"id":"Q29tbWVudDo3"},{"id":"Q29tbWVudDo4"},{"id":"Q29tbWVudDo5"}]}}],"completed":[{"id":"1"},{"id":"2"},{"id":"3"}]}
    a: query($_v0:ID!){user(id: $_v0){__typename id}} {"_v0":"1"}
    b: query($representations:[_Any!]!, $postLimit:Int!){_entities(representations: $representations){...on User{posts(first: $postLimit){__typename id}}}} {"postLimit":3,"representations":[{"__typename":"User","id":"VXNlcjox"}]}
    c: query($representations:[_Any!]!, $commentLimit:Int!){_entities(representations: $representations){...on Post{comments(first: $commentLimit){id}}}} {"commentLimit":2,"representations":[{"__typename":"Post","id":"UG9zdDox"},{"__typename":"Post","id":"UG9zdDoy"},{"__typename":"Post","id":"UG9zdDoz"}]}
    "#);
}

#[ntex::test]
async fn forwarded_variable_over_a_list() {
    let setup = start("posts", posts_mocks()).await;
    let parts = setup
        .run(
            r#"
              query GetUsersPosts($limit: Int!) {
                users {
                  id
                  ... @defer(label: "posts") { posts(first: $limit) { id } }
                }
              }
            "#,
            Some(json!({ "limit": 5 })),
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{"id":"VXNlcjox"},{"id":"VXNlcjoy"},{"id":"VXNlcjoz"}]},"pending":[{"id":"0","path":["users",0],"label":"posts"},{"id":"1","path":["users",1],"label":"posts"},{"id":"2","path":["users",2],"label":"posts"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"posts":[{"id":"UG9zdDox"},{"id":"UG9zdDoy"},{"id":"UG9zdDoz"}]}},{"id":"1","data":{"posts":[{"id":"UG9zdDo0"},{"id":"UG9zdDo1"},{"id":"UG9zdDo2"}]}},{"id":"2","data":{"posts":[{"id":"UG9zdDo3"},{"id":"UG9zdDo4"},{"id":"UG9zdDo5"}]}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    a: {users{__typename id}}
    b: query($representations:[_Any!]!, $limit:Int!){_entities(representations: $representations){...on User{posts(first: $limit){id}}}} {"limit":5,"representations":[{"__typename":"User","id":"VXNlcjox"},{"__typename":"User","id":"VXNlcjoy"},{"__typename":"User","id":"VXNlcjoz"}]}
    "#);
}

/// The mutation runs exactly once.
#[ntex::test]
async fn mutation_runs_once() {
    let setup = start(
        "mutation",
        json!({
            "a": {
                "mutation": { "createProduct": { "__typename": "Product", "id": "UHJvZHVjdDox", "name": "Product: UHJvZHVjdDox" } }
            },
            "b": {
                "entities": [{ "__typename": "Product", "id": "UHJvZHVjdDox", "price": 123.456 }]
            }
        }),
    )
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
    {"data":{"createProduct":{"name":"Product: UHJvZHVjdDox"}},"pending":[{"id":"0","path":["createProduct"],"label":"pricing"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"price":123.456}}],"completed":[{"id":"0"}]}
    a: mutation($_v0:String!){createProduct(name: $_v0){__typename name id}} {"_v0":"Widget"}
    b: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price}}} {"representations":[{"__typename":"Product","id":"UHJvZHVjdDox"}]}
    "#);
}

fn accounts_reviews_mocks() -> Value {
    let user = |id: &str| json!({ "__typename": "User", "id": id, "name": format!("User: {id}"), "birthdate": format!("User: {id}") });
    json!({
        "accounts": {
            "query": { "users": [user("VXNlcjox"), user("VXNlcjoy"), user("VXNlcjoz")] },
            "entities": [user("VXNlcjox"), user("VXNlcjoy"), user("VXNlcjoz")]
        },
        "reviews": {
            "entities": [
                { "__typename": "User", "id": "VXNlcjox", "reviewCount": 123 },
                { "__typename": "User", "id": "VXNlcjoy", "reviewCount": 123 },
                { "__typename": "User", "id": "VXNlcjoz", "reviewCount": 123 }
            ]
        }
    })
}

/// `accounts` has both `users` and `userById`. Only one `users` call.
#[ntex::test]
async fn list_anchor_uses_a_keyed_lookup() {
    let setup = start("accounts-reviews", accounts_reviews_mocks()).await;
    let parts = setup
        .run(
            "{ users { ... @defer { id name } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{},{},{}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"id":"VXNlcjox","name":"User: VXNlcjox"}},{"id":"1","data":{"id":"VXNlcjoy","name":"User: VXNlcjoy"}},{"id":"2","data":{"id":"VXNlcjoz","name":"User: VXNlcjoz"}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{id name}}} {"representations":[{"__typename":"User","id":"VXNlcjox"},{"__typename":"User","id":"VXNlcjoy"},{"__typename":"User","id":"VXNlcjoz"}]}
    accounts: {users{__typename id}}
    "#);
}

#[ntex::test]
async fn same_and_other_subgraph_fields_are_both_kept() {
    let setup = start("accounts-reviews", accounts_reviews_mocks()).await;
    let parts = setup
        .run(
            "{ users { name ... @defer { birthdate reviewCount } } }",
            None,
            Plain::Stripped,
        )
        .await;
    assert_snapshot!(parts, @r#"
    {"data":{"users":[{"name":"User: VXNlcjox"},{"name":"User: VXNlcjoy"},{"name":"User: VXNlcjoz"}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",1]},{"id":"2","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"birthdate":"User: VXNlcjox","reviewCount":123}},{"id":"1","data":{"birthdate":"User: VXNlcjoy","reviewCount":123}},{"id":"2","data":{"birthdate":"User: VXNlcjoz","reviewCount":123}}],"completed":[{"id":"0"},{"id":"1"},{"id":"2"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{birthdate}}} {"representations":[{"__typename":"User","id":"VXNlcjox"},{"__typename":"User","id":"VXNlcjoy"},{"__typename":"User","id":"VXNlcjoz"}]}
    accounts: {users{__typename name id}}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviewCount}}} {"representations":[{"__typename":"User","id":"VXNlcjox"},{"__typename":"User","id":"VXNlcjoy"},{"__typename":"User","id":"VXNlcjoz"}]}
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
    let subgraphs = TestSubgraphs::builder()
        .with_path_delay("/inventory", std::time::Duration::from_millis(500))
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
    let subgraphs = TestSubgraphs::builder()
        .with_path_delay("/inventory", std::time::Duration::from_millis(500))
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

    let subgraphs = TestSubgraphs::builder()
        .with_path_delay("/reviews", Duration::from_millis(1000))
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
