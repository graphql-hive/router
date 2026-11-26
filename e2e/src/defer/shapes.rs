//! The same kinds of `@defer` over five base queries on the demo supergraph. Every case checks
//! that the merged parts are the same as the response without `@defer`, some also snapshot the
//! parts and the subgraph calls.
//!
//! The bases:
//! - `top`: a root list
//! - `me`: a single entity
//! - `nested`: entities nested across three subgraphs (a list inside a list)
//! - `requires`: `@requires` (inventory's `shippingEstimate`) on a root list
//! - `requires_deep`: `@requires` a few levels down
//!
//! The demo has no interfaces, unions or real `[[T]]` fields, `paths` covers those.

use insta::assert_snapshot;

use super::*;

async fn check(query: &str) -> Run {
    // some shapes defer every field, more than `limits.max_defers` lets through by default
    let setup = start_full("supergraph.graphql", None, "limits:\n  max_defers: null\n").await;
    let run = setup.check(query, None).await;
    assert!(
        run.parts.len() > 1,
        "expected an incremental response, got {:?}",
        run.parts
    );
    run
}

// Cases that only check merged == plain. The snapshotted ones are written out further down.
macro_rules! cases {
    ($($name:ident => $query:expr;)*) => {
        $(
            #[ntex::test]
            async fn $name() {
                check($query).await;
            }
        )*
    };
}

// top: { topProducts(first: 2) { upc name price inStock reviews { id } } }
cases! {
    top_defer_one_field => r#"{ topProducts(first: 2) { upc name price ... @defer { inStock } reviews { id } } }"#;
    top_defer_between_fields => r#"{ topProducts(first: 2) { upc ... @defer { name } price inStock reviews { id } } }"#;
    top_defer_field_group => r#"{ topProducts(first: 2) { upc ... @defer { name price inStock } reviews { id } } }"#;
    top_defer_whole_query => r#"{ ... @defer { topProducts(first: 2) { upc name price inStock reviews { id } } } }"#;
    top_defer_nested_group => r#"{ topProducts(first: 2) { upc ... @defer { name price ... @defer { inStock reviews { id } } } } }"#;
    top_defer_query_and_nested_fields => r#"{ ... @defer { topProducts(first: 2) { upc name ... @defer { price inStock reviews { id } } } } me { id } }"#;
    top_defer_child_object => r#"{ topProducts(first: 2) { upc name price inStock ... @defer { reviews { id } } } }"#;
    top_defer_duplicate_selection => r#"{ topProducts(first: 2) { upc name inStock ... @defer { name price inStock } reviews { id } } }"#;
    top_defer_fragment_with_nested_fragment => r#"
        query { topProducts(first: 2) { upc ... @defer { ...ProductDetails } } }
        fragment ProductDetails on Product { name price ... @defer { inStock reviews { id } } }
    "#;
    top_defer_inside_fragment => r#"
        query { topProducts(first: 2) { ...ProductFields reviews { id } } }
        fragment ProductFields on Product { upc ... @defer { name price inStock } }
    "#;
    top_defer_fragment_spread => r#"
        query { topProducts(first: 2) { upc ...ProductFields @defer reviews { id } } }
        fragment ProductFields on Product { name price inStock }
    "#;
}

// nested: { topProducts(first: 1) { upc name reviews { id author { id name username } } } }
cases! {
    nested_defer_one_field => r#"{ topProducts(first: 1) { upc name reviews { id author { id username ... @defer { name } } } } }"#;
    nested_defer_between_fields => r#"{ topProducts(first: 1) { upc ... @defer { name } reviews { id author { id name username } } } }"#;
    nested_defer_field_group => r#"{ topProducts(first: 1) { upc name reviews { id author { id ... @defer { name username } } } } }"#;
    nested_defer_whole_query => r#"{ ... @defer { topProducts(first: 1) { upc name reviews { id author { id name username } } } } }"#;
    nested_defer_nested_group => r#"{ topProducts(first: 1) { upc ... @defer { name reviews { id ... @defer { author { id name username } } } } } }"#;
    nested_defer_query_and_nested_fields => r#"{ ... @defer { topProducts(first: 1) { upc name reviews { id ... @defer { author { id name } } } } } me { id } }"#;
    nested_defer_child_object => r#"{ topProducts(first: 1) { upc name reviews { id ... @defer { author { id name username } } } } }"#;
    nested_defer_duplicate_selection => r#"{ topProducts(first: 1) { upc name reviews { id author { id name } ... @defer { author { name username } } } } }"#;
    nested_defer_fragment_with_nested_fragment => r#"
        query { topProducts(first: 1) { upc ... @defer { ...ProductDetails } } }
        fragment ProductDetails on Product { name reviews { id ... @defer { author { id name username } } } }
    "#;
    nested_defer_inside_fragment => r#"
        query { topProducts(first: 1) { upc name reviews { ...ReviewFields } } }
        fragment ReviewFields on Review { id author { ... @defer { id name } username } }
    "#;
    nested_defer_fragment_spread => r#"
        query { topProducts(first: 1) { upc name reviews { id author { id ...UserFields @defer } } } }
        fragment UserFields on User { name username }
    "#;
}

// requires: { topProducts(first: 2) { upc name inStock shippingEstimate } }
cases! {
    requires_defer_one_field => r#"{ topProducts(first: 2) { upc name inStock ... @defer { shippingEstimate } } }"#;
    requires_defer_between_fields => r#"{ topProducts(first: 2) { upc ... @defer { shippingEstimate } name inStock } }"#;
    requires_defer_field_group => r#"{ topProducts(first: 2) { upc name ... @defer { inStock shippingEstimate } } }"#;
    requires_defer_whole_query => r#"{ ... @defer { topProducts(first: 2) { upc name inStock shippingEstimate } } }"#;
    requires_defer_nested_group => r#"{ topProducts(first: 2) { upc ... @defer { name ... @defer { inStock shippingEstimate } } } }"#;
    requires_defer_query_and_nested_fields => r#"{ ... @defer { topProducts(first: 2) { upc name ... @defer { inStock shippingEstimate } } } me { id } }"#;
    requires_defer_child_object => r#"{ topProducts(first: 2) { upc name inStock shippingEstimate ... @defer { reviews { id product { upc shippingEstimate } } } } }"#;
    requires_defer_duplicate_selection => r#"{ topProducts(first: 2) { upc name shippingEstimate ... @defer { inStock shippingEstimate } } }"#;
    requires_defer_fragment_with_nested_fragment => r#"
        query { topProducts(first: 2) { upc ... @defer { ...Shipping } } }
        fragment Shipping on Product { name ... @defer { inStock shippingEstimate } }
    "#;
    requires_defer_inside_fragment => r#"
        query { topProducts(first: 2) { ...Shipping } }
        fragment Shipping on Product { upc name ... @defer { shippingEstimate } }
    "#;
    requires_defer_fragment_spread => r#"
        query { topProducts(first: 2) { upc name ...Shipping @defer } }
        fragment Shipping on Product { inStock shippingEstimate }
    "#;
}

// requires_deep: { me { id reviews { id product { upc name shippingEstimate } } } }
cases! {
    requires_deep_defer_one_field => r#"{ me { id reviews { id product { upc name ... @defer { shippingEstimate } } } } }"#;
    requires_deep_defer_between_fields => r#"{ me { id reviews { id product { upc ... @defer { shippingEstimate } name } } } }"#;
    requires_deep_defer_field_group => r#"{ me { id reviews { id product { upc ... @defer { name shippingEstimate } } } } }"#;
    requires_deep_defer_whole_query => r#"{ ... @defer { me { id reviews { id product { upc name shippingEstimate } } } } }"#;
    requires_deep_defer_nested_group => r#"{ me { id ... @defer { reviews { id product { upc ... @defer { name shippingEstimate } } } } } }"#;
    requires_deep_defer_query_and_nested_fields => r#"{ ... @defer { me { id reviews { id product { upc ... @defer { shippingEstimate } } } } } topProducts(first: 1) { upc } }"#;
    requires_deep_defer_child_object => r#"{ me { id reviews { id ... @defer { product { upc name shippingEstimate } } } } }"#;
    requires_deep_defer_duplicate_selection => r#"{ me { id reviews { id product { upc shippingEstimate ... @defer { name shippingEstimate } } } } }"#;
    requires_deep_defer_fragment_with_nested_fragment => r#"
        query { me { id ... @defer { ...Reviews } } }
        fragment Reviews on User { reviews { id product { upc ... @defer { shippingEstimate } } } }
    "#;
    requires_deep_defer_inside_fragment => r#"
        query { me { id reviews { id product { ...Shipping } } } }
        fragment Shipping on Product { upc ... @defer { name shippingEstimate } }
    "#;
    requires_deep_defer_fragment_spread => r#"
        query { me { id reviews { id product { upc ...Shipping @defer } } } }
        fragment Shipping on Product { name shippingEstimate }
    "#;
}

// me: { me { id name username reviews { id product { upc name } } } }
// Every shape gets a snapshot, the data is small.

#[ntex::test]
async fn me_defer_one_field() {
    let d =
        check(r#"{ me { id name username reviews { id product { upc ... @defer { name } } } } }"#)
            .await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1","name":"Uri Goldshtein","username":"urigo","reviews":[{"id":"1","product":{"upc":"1"}},{"id":"2","product":{"upc":"1"}}]}},"pending":[{"id":"0","path":["me","reviews",0,"product"]},{"id":"1","path":["me","reviews",1,"product"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Table"}},{"id":"1","data":{"name":"Table"}}],"completed":[{"id":"0"},{"id":"1"}]}
    accounts: {me{__typename id name username}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_between_fields() {
    let d =
        check(r#"{ me { id ... @defer { name } username reviews { id product { upc name } } } }"#)
            .await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1","username":"urigo","reviews":[{"id":"1","product":{"upc":"1","name":"Table"}},{"id":"2","product":{"upc":"1","name":"Table"}}]}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"0"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id username}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_field_group() {
    let d =
        check(r#"{ me { id ... @defer { name username reviews { id product { upc name } } } } }"#)
            .await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein","username":"urigo","reviews":[{"id":"1","product":{"upc":"1","name":"Table"}},{"id":"2","product":{"upc":"1","name":"Table"}}]}}],"completed":[{"id":"0"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name username}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_whole_query() {
    let d =
        check(r#"{ ... @defer { me { id name username reviews { id product { upc name } } } } }"#)
            .await;
    assert_snapshot!(d.text, @r#"
    {"data":{},"pending":[{"id":"0","path":[]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"me":{"id":"1","name":"Uri Goldshtein","username":"urigo","reviews":[{"id":"1","product":{"upc":"1","name":"Table"}},{"id":"2","product":{"upc":"1","name":"Table"}}]}}}],"completed":[{"id":"0"}]}
    accounts: {me{__typename id name username}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_nested_group() {
    let d = check(r#"{ me { id username ... @defer { name ... @defer { reviews { id product { upc name } } } } } }"#).await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1","username":"urigo"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"pending":[{"id":"1","path":["me"]}],"incremental":[{"id":"0","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"reviews":[{"id":"1","product":{"upc":"1","name":"Table"}},{"id":"2","product":{"upc":"1","name":"Table"}}]}}],"completed":[{"id":"1"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id username}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_query_and_nested_fields() {
    let d = check(r#"{ me { id } ... @defer { me { name username reviews { id ... @defer { product { upc name } } } } } }"#).await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":[]}],"hasNext":true}
    {"pending":[{"id":"1","path":["me","reviews",0]},{"id":"2","path":["me","reviews",1]}],"incremental":[{"id":"0","data":{"me":{"name":"Uri Goldshtein","username":"urigo","reviews":[{"id":"1"},{"id":"2"}]}}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"product":{"upc":"1","name":"Table"}}},{"id":"2","data":{"product":{"upc":"1","name":"Table"}}}],"completed":[{"id":"1"},{"id":"2"}]}
    accounts: {me{__typename name username id}}
    accounts: {me{id}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on Review{product{__typename upc}}}} {"representations":[{"__typename":"Review","id":"1"},{"__typename":"Review","id":"2"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{__typename id}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_sibling_defers() {
    let d = check(r#"{ me { id ... @defer { name username } ... @defer { reviews { id product { upc name } } } } }"#).await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]},{"id":"1","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein","username":"urigo"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"reviews":[{"id":"1","product":{"upc":"1","name":"Table"}},{"id":"2","product":{"upc":"1","name":"Table"}}]}}],"completed":[{"id":"1"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name username}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_child_object() {
    let d =
        check(r#"{ me { id name username ... @defer { reviews { id product { upc name } } } } }"#)
            .await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1","name":"Uri Goldshtein","username":"urigo"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"reviews":[{"id":"1","product":{"upc":"1","name":"Table"}},{"id":"2","product":{"upc":"1","name":"Table"}}]}}],"completed":[{"id":"0"}]}
    accounts: {me{__typename id name username}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_duplicate_selection() {
    let d = check(
        r#"{ me { id name ... @defer { name username } reviews { id product { upc name } } } }"#,
    )
    .await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1","name":"Uri Goldshtein","reviews":[{"id":"1","product":{"upc":"1","name":"Table"}},{"id":"2","product":{"upc":"1","name":"Table"}}]}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"username":"urigo"}}],"completed":[{"id":"0"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name username}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id name}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_every_field() {
    let d = check(
        r#"{ ... @defer { me {
            ... @defer { id } ... @defer { name } ... @defer { username }
            ... @defer { reviews { ... @defer { id } ... @defer { product { ... @defer { upc } ... @defer { name } } } } }
        } } }"#,
    )
    .await;
    assert_snapshot!(d.text, @r#"
    {"data":{},"pending":[{"id":"0","path":[]}],"hasNext":true}
    {"pending":[{"id":"1","path":["me"]},{"id":"2","path":["me"]},{"id":"3","path":["me"]},{"id":"4","path":["me"]}],"incremental":[{"id":"0","data":{"me":{}}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"id":"1"}}],"completed":[{"id":"1"}]}
    {"incremental":[{"id":"2","data":{"name":"Uri Goldshtein"}}],"completed":[{"id":"2"}]}
    {"incremental":[{"id":"3","data":{"username":"urigo"}}],"completed":[{"id":"3"}]}
    {"pending":[{"id":"5","path":["me","reviews",0]},{"id":"6","path":["me","reviews",1]},{"id":"7","path":["me","reviews",0]},{"id":"8","path":["me","reviews",1]}],"incremental":[{"id":"4","data":{"reviews":[{},{}]}}],"completed":[{"id":"4"}]}
    {"incremental":[{"id":"5","data":{"id":"1"}},{"id":"6","data":{"id":"2"}}],"completed":[{"id":"5"},{"id":"6"}]}
    {"pending":[{"id":"9","path":["me","reviews",0,"product"]},{"id":"10","path":["me","reviews",0,"product"]},{"id":"11","path":["me","reviews",1,"product"]},{"id":"12","path":["me","reviews",1,"product"]}],"incremental":[{"id":"7","data":{"product":{}}},{"id":"8","data":{"product":{}}}],"completed":[{"id":"7"},{"id":"8"}]}
    {"incremental":[{"id":"10","data":{"name":"Table"}},{"id":"12","data":{"name":"Table"}}],"completed":[{"id":"10"},{"id":"12"}]}
    {"incremental":[{"id":"9","data":{"upc":"1"}},{"id":"11","data":{"upc":"1"}}],"completed":[{"id":"9"},{"id":"11"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{username}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on Review{product{__typename upc}}}} {"representations":[{"__typename":"Review","id":"1"},{"__typename":"Review","id":"2"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{__typename id}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_fragment_with_nested_fragment() {
    let d = check(
        r#"
        query { me { id ... @defer { ...UserDetails } } }
        fragment UserDetails on User { name username ... @defer { reviews { id product { upc name } } } }
        "#,
    )
    .await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"pending":[{"id":"1","path":["me"]}],"incremental":[{"id":"0","data":{"name":"Uri Goldshtein","username":"urigo"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"reviews":[{"id":"1","product":{"upc":"1","name":"Table"}},{"id":"2","product":{"upc":"1","name":"Table"}}]}}],"completed":[{"id":"1"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name username}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_inside_fragment() {
    let d = check(
        r#"
        query { me { ...UserFields } }
        fragment UserFields on User { id ... @defer { name username } reviews { id product { ...ProductFields } } }
        fragment ProductFields on Product { upc ... @defer { name } }
        "#,
    )
    .await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1","reviews":[{"id":"1","product":{"upc":"1"}},{"id":"2","product":{"upc":"1"}}]}},"pending":[{"id":"0","path":["me"]},{"id":"1","path":["me","reviews",0,"product"]},{"id":"2","path":["me","reviews",1,"product"]}],"hasNext":true}
    {"incremental":[{"id":"1","data":{"name":"Table"}},{"id":"2","data":{"name":"Table"}}],"completed":[{"id":"1"},{"id":"2"}]}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein","username":"urigo"}}],"completed":[{"id":"0"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name username}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

#[ntex::test]
async fn me_defer_fragment_spread() {
    let d = check(
        r#"
        query { me { id ...UserFields @defer reviews { id product { ...ProductFields @defer } } } }
        fragment UserFields on User { name username }
        fragment ProductFields on Product { upc name }
        "#,
    )
    .await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1","reviews":[{"id":"1","product":{}},{"id":"2","product":{}}]}},"pending":[{"id":"0","path":["me"]},{"id":"1","path":["me","reviews",0,"product"]},{"id":"2","path":["me","reviews",1,"product"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Uri Goldshtein","username":"urigo"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"upc":"1","name":"Table"}},{"id":"2","data":{"upc":"1","name":"Table"}}],"completed":[{"id":"1"},{"id":"2"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name username}}} {"representations":[{"__typename":"User","id":"1"}]}
    accounts: {me{__typename id}}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}

// Sibling defers and "defer every field" on the other bases.

#[ntex::test]
async fn top_sibling_defers() {
    let d = check(r#"{ topProducts(first: 2) { upc ... @defer { name price } ... @defer { inStock } reviews { id } } }"#).await;
    assert_snapshot!(d.text, @r#"
    {"data":{"topProducts":[{"upc":"1","reviews":[{"id":"1"},{"id":"2"},{"id":"3"},{"id":"4"}]},{"upc":"2","reviews":[{"id":"5"},{"id":"6"},{"id":"7"},{"id":"8"}]}]},"pending":[{"id":"0","path":["topProducts",0]},{"id":"1","path":["topProducts",1]},{"id":"2","path":["topProducts",0]},{"id":"3","path":["topProducts",1]}],"hasNext":true}
    {"incremental":[{"id":"2","data":{"inStock":true}},{"id":"3","data":{"inStock":false}}],"completed":[{"id":"2"},{"id":"3"}]}
    {"incremental":[{"id":"0","data":{"name":"Table","price":899}},{"id":"1","data":{"name":"Couch","price":1299}}],"completed":[{"id":"0"},{"id":"1"}]}
    inventory: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{inStock}}} {"representations":[{"__typename":"Product","upc":"1"},{"__typename":"Product","upc":"2"}]}
    products: query($_v0:Int){topProducts(first: $_v0){__typename upc}} {"_v0":2}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name price}}} {"representations":[{"__typename":"Product","upc":"1"},{"__typename":"Product","upc":"2"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reviews{id}}}} {"representations":[{"__typename":"Product","upc":"1"},{"__typename":"Product","upc":"2"}]}
    "#);
}

#[ntex::test]
async fn nested_sibling_defers() {
    let d = check(r#"{ topProducts(first: 1) { upc ... @defer { name } ... @defer { reviews { id author { id name username } } } } }"#).await;
    assert_snapshot!(d.text, @r#"
    {"data":{"topProducts":[{"upc":"1"}]},"pending":[{"id":"0","path":["topProducts",0]},{"id":"1","path":["topProducts",0]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Table"}}],"completed":[{"id":"0"}]}
    {"incremental":[{"id":"1","data":{"reviews":[{"id":"1","author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}},{"id":"2","author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}},{"id":"3","author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}},{"id":"4","author":{"id":"1","name":"Uri Goldshtein","username":"urigo"}}]}}],"completed":[{"id":"1"}]}
    accounts: query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}} {"representations":[{"__typename":"User","id":"1"}]}
    products: query($_v0:Int){topProducts(first: $_v0){__typename upc}} {"_v0":1}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reviews{id author{__typename id username}}}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    "#);
}

#[ntex::test]
async fn requires_sibling_defers() {
    let d = check(r#"{ topProducts(first: 2) { upc name ... @defer { inStock } ... @defer { shippingEstimate } } }"#).await;
    assert_snapshot!(d.text, @r#"
    {"data":{"topProducts":[{"upc":"1","name":"Table"},{"upc":"2","name":"Couch"}]},"pending":[{"id":"0","path":["topProducts",0]},{"id":"1","path":["topProducts",1]},{"id":"2","path":["topProducts",0]},{"id":"3","path":["topProducts",1]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"inStock":true}},{"id":"1","data":{"inStock":false}}],"completed":[{"id":"0"},{"id":"1"}]}
    {"incremental":[{"id":"2","data":{"shippingEstimate":50}},{"id":"3","data":{"shippingEstimate":0}}],"completed":[{"id":"2"},{"id":"3"}]}
    inventory: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{inStock}}} {"representations":[{"__typename":"Product","upc":"1"},{"__typename":"Product","upc":"2"}]}
    inventory: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate}}} {"representations":[{"__typename":"Product","price":899,"weight":100,"upc":"1"},{"__typename":"Product","price":1299,"weight":1000,"upc":"2"}]}
    products: query($_v0:Int){topProducts(first: $_v0){__typename upc name}} {"_v0":2}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price weight}}} {"representations":[{"__typename":"Product","upc":"1"},{"__typename":"Product","upc":"2"}]}
    "#);
}

#[ntex::test]
async fn requires_deep_sibling_defers() {
    let d = check(r#"{ me { id reviews { id product { upc ... @defer { name } ... @defer { shippingEstimate } } } } }"#).await;
    assert_snapshot!(d.text, @r#"
    {"data":{"me":{"id":"1","reviews":[{"id":"1","product":{"upc":"1"}},{"id":"2","product":{"upc":"1"}}]}},"pending":[{"id":"0","path":["me","reviews",0,"product"]},{"id":"1","path":["me","reviews",1,"product"]},{"id":"2","path":["me","reviews",0,"product"]},{"id":"3","path":["me","reviews",1,"product"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Table"}},{"id":"1","data":{"name":"Table"}}],"completed":[{"id":"0"},{"id":"1"}]}
    {"incremental":[{"id":"2","data":{"shippingEstimate":50}},{"id":"3","data":{"shippingEstimate":50}}],"completed":[{"id":"2"},{"id":"3"}]}
    accounts: {me{__typename id}}
    inventory: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate}}} {"representations":[{"__typename":"Product","price":899,"weight":100,"upc":"1"}]}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{name}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    products: query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price weight}}} {"representations":[{"__typename":"Product","upc":"1"}]}
    reviews: query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id product{__typename upc}}}}} {"representations":[{"__typename":"User","id":"1"}]}
    "#);
}
