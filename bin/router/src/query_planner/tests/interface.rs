use crate::query_planner::{
    tests::testkit::{
        build_query_plan_from_sdl, build_query_plan_with_defaults, init_logger, read_supergraph,
    },
    utils::parsing::parse_operation,
};
use std::error::Error;

/// Tests querying the `node` interface field using two different aliases (`account` and `chat`).
/// Verifies that aliases work correctly when querying the same interface field with different IDs.
#[test]
fn node_query_with_aliases_on_interface_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account: node(id: "a1") {
            __typename
          }
          chat: node(id: "c1") {
            __typename
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { account: node(id: "a1") { ...a } chat: node(id: "c1") { ...a } }
        fragment a on Node { __typename }
    "#);

    Ok(())
}

/// Tests querying the `node` interface field with inline fragments for concrete object types (`Account` and `Chat`).
/// Verifies that inline fragments on specific object types are handled correctly when returned from an interface field.
#[test]
fn node_query_with_inline_fragments_on_object_types() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account: node(id: "a1") {
            ... on Account {
              id
              username
            }
          }
          chat: node(id: "c1") {
            ... on Chat {
              id
              text
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [3] b               -
      [2] a               -

    [3] b
      out
        { chat: node(id: "c1") { __typename ... on Chat { id text } } }

    [2] a
      out
        { account: node(id: "a1") { __typename ... on Account { id username } } }
    "#);

    Ok(())
}

/// Tests querying the `node` interface field with inline fragments for types that do not match the actual type of the returned object.
/// Verifies that fragments for non-matching types do not cause errors and are handled gracefully.
#[test]
fn node_query_with_cross_type_inline_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account: node(id: "a1") {
            ... on Chat {
              id
            }
          }
          chat: node(id: "c1") {
            ... on Account {
              id
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [3] a               -
      [2] b               -

    [3] a
      out
        { chat: node(id: "c1") { __typename ... on Account { id } } }

    [2] b
      out
        { account: node(id: "a1") { __typename ... on Chat { id } } }
    "#);

    Ok(())
}

/// Tests querying the `node` interface field with both `__typename` and cross-type inline fragments.
/// Verifies that `__typename` is always available and that fragments for non-matching types are handled correctly.
#[test]
fn node_query_with_typename_and_cross_type_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account: node(id: "a1") {
            __typename
            ... on Chat {
              id
            }
          }
          chat: node(id: "c1") {
            __typename
            ... on Account {
              id
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [3] a               -
      [2] b               -

    [3] a
      out
        { chat: node(id: "c1") { __typename ... on Account { id } } }

    [2] b
      out
        { account: node(id: "a1") { __typename ... on Chat { id } } }
    "#);

    Ok(())
}

/// Tests querying the `chat` object field directly by ID.
/// Verifies that direct object type queries (not through an interface) are resolved as expected.
#[test]
fn direct_object_query_chat_by_id() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          chat(id: "c1") {
            id
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -

    [2] b
      out
        { chat(id: "c1") { id } }
    "#);

    Ok(())
}

/// Tests querying the `account` object field directly by ID.
/// Verifies that direct object type queries (not through an interface) are resolved as expected.
#[test]
fn direct_object_query_account_by_id() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account(id: "a1") {
            id
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { account(id: "a1") { id } }
    "#);

    Ok(())
}

/// Tests querying the `chat` object field with a nested `account` object field.
/// Verifies that nested object fields are resolved correctly in the query plan.
#[test]
fn object_query_with_nested_object_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          chat(id: "c1") {
            id
            text
            account {
              id
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] b               -
      [3] a               [2]

    [2] b
      out
        { chat(id: "c1") { __typename id text } }

    [3] a   after [2]
      merge
        chat
      in
        ... on Chat { __typename id }
      out
        ... on Chat { account { id } }
    "#);

    Ok(())
}

/// Tests querying the `account` object field with a nested `chats` list field.
/// Verifies that nested list fields are resolved correctly in the query plan.
#[test]
fn object_query_with_nested_list_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          account(id: "a1") {
            id
            username
            chats {
              id
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] a               -
      [3] b               [2]

    [2] a
      out
        { account(id: "a1") { __typename id username } }

    [3] b   after [2]
      merge
        account
      in
        ... on Account { __typename id }
      out
        ... on Account { chats { id } }
    "#);

    Ok(())
}

/// Tests querying the `node` interface field for the `id` field directly.
/// Verifies that an error is returned if no subgraph resolves `id` for all object types implementing the interface.
#[test]
fn node_query_with_id_on_interface_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          node(id: "a1") {
            id
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    );

    // By definition @shareable means: QP can pick any field to resolve data, it shouldn't matter which one is used.
    // Performing type expansion and fetching data from two subgraphs breaks that rule.
    assert!(query_plan.is_err());

    Ok(())
}

/// Tests querying the `node` interface field with multiple inline fragments for different possible types (`Chat` and `Account`).
/// Verifies that an error is returned if the required fields cannot be resolved for all possible types.
#[test]
fn node_query_with_multiple_type_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          node(id: "a1") {
            ... on Chat {
              id
            }
            ... on Account {
              id
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    );

    assert!(query_plan.is_err());

    Ok(())
}

/// Tests querying the `node` interface field with both a direct `id` field and an inline fragment for a mismatched type.
/// Verifies that an error is returned if the required fields cannot be resolved for all possible types.
#[test]
fn node_query_with_id_and_cross_type_fragment_overlap() -> Result<(), Box<dyn Error>> {
    init_logger();
    // By definition @shareable means: QP can pick any field to resolve data, it shouldn't matter which one is used.
    // Performing type expansion and fetching data from two subgraphs breaks that rule.
    let document = parse_operation(
        r#"
        query {
          account: node(id: "a1") {
            id
            ... on Chat {
              id
            }
          }
          chat: node(id: "c1") {
            __typename
            ... on Account {
              id
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        document,
    );

    assert!(query_plan.is_err());

    Ok(())
}

#[test]
fn type_expand_interface_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          products {
            id
            reviews {
              id
            }
          }
        }
      ,
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/abstract-types.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] products        -
      [4] reviews         [2]

    [2] products
      out
        { products { __typename id } }

    [4] reviews   after [2]
      merge
        products.@
      in
        ... on Book { __typename id } ... on Magazine { __typename id }
      out
        ... on Book { reviews { ...a } } ... on Magazine { reviews { ...a } }
        fragment a on Review { id }
    ");

    Ok(())
}

#[test]
fn requires_on_field_with_args_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          book: similar(id: "p1") {
            id
            sku
            delivery(zip: "1234") {
              fastestDelivery
              estimatedDelivery
            }
          }
          magazine: similar(id: "p2") {
            id
            sku
            delivery(zip: "1234") {
              fastestDelivery
              estimatedDelivery
            }
          }
        }
      ,
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/abstract-types.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    fetches
      fetch               after
      [2] products        -
      [16] inventory      [2]

    [2] products
      out
        { book: similar(id: "p1") { ...a } magazine: similar(id: "p2") { ...a } }
        fragment a on Product { __typename id sku dimensions { weight size } }

    [16] inventory   after [2]
      alias _e0
        merge
          magazine.@
          book.@
        in
          ... on Book { __typename dimensions { size weight } id }
          ... on Magazine { __typename dimensions { size weight } id }
        out
          ... on Book { delivery(zip: "1234") { ...a } }
          ... on Magazine { delivery(zip: "1234") { ...a } }
      fragments
        fragment a on DeliveryEstimates {
          ... on DeliveryEstimates { fastestDelivery estimatedDelivery }
        }
    "#);

    Ok(())
}

#[test]
fn nested_interface_field_with_inline_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($title: Boolean = true) {
          products {
            id
            reviews {
              product {
                id
                ... on Book @include(if: $title) {
                  title
                }
                ... on Magazine {
                  sku
                }
              }
            }
          }
        }
      ,
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/abstract-types.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $title

    fetches
      fetch               after
      [2] products        -
      [6] reviews         [2]
      [4] books           [6]
      [5] products        [6]

    [2] products
      out
        { products { __typename id } }

    [6] reviews   after [2]
      merge
        products.@
      in
        ... on Book { __typename id } ... on Magazine { __typename id }
      out
        ... on Book { reviews { ...a } } ... on Magazine { reviews { ...a } }
        fragment a on Review { product { __typename id } }

    [4] books   after [6]
      if
        $title
      merge
        products.@.reviews.@.product|[Book]
      in
        ... on Book { __typename id }
      out
        ... on Book { title }

    [5] products   after [6]
      merge
        products.@.reviews.@.product|[Magazine]
      in
        ... on Magazine { __typename id }
      out
        ... on Magazine { sku }
    ");

    Ok(())
}

#[test]
fn nested_interface_field_with_redundant_inline_fragments() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($title: Boolean = true) {
          products {
            id
            reviews {
              product {
                id
                ... on Book @include(if: $title) {
                  title
                  ... on Book {
                    sku
                  }
                }
                ... on Magazine {
                  sku
                }
              }
            }
          }
        }
      ,
      "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/abstract-types.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $title

    fetches
      fetch               after
      [2] products        -
      [7] reviews         [2]
      [4] books           [7]
      [5] products        [7]

    [2] products
      out
        { products { __typename id } }

    [7] reviews   after [2]
      merge
        products.@
      in
        ... on Book { __typename id } ... on Magazine { __typename id }
      out
        ... on Book { reviews { ...a } } ... on Magazine { reviews { ...a } }
        fragment a on Review { product { __typename id } }

    [4] books   after [7]
      if
        $title
      merge
        products.@.reviews.@.product|[Book]
      in
        ... on Book { __typename id }
      out
        ... on Book { title }

    [5] products   after [7]
      alias _e0
        if
          $title
        merge
          products.@.reviews.@.product|[Book]
        in
          ... on Book { __typename id }
        out
          ... on Book { sku }
      alias _e1
        merge
          products.@.reviews.@.product|[Magazine]
        in
          ... on Magazine { __typename id }
        out
          ... on Magazine { sku }
    ");

    Ok(())
}

/// Same as above, but `Book` is renamed to `Tome` so it sorts after `Magazine`. Batching then
/// declares an empty `Magazine` slot on the conditional `Tome` step before merging, and that slot
/// used to get its own `... on Magazine @include(if: $title) { }`, an empty selection set.
#[test]
fn nested_interface_field_with_redundant_inline_fragments_other_type_order(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let supergraph =
        read_supergraph("fixture/tests/abstract-types.supergraph.graphql").replace("Book", "Tome");
    let document = parse_operation(
        r#"
        query ($title: Boolean = true) {
          products {
            id
            reviews {
              product {
                id
                ... on Tome @include(if: $title) {
                  title
                  ... on Tome {
                    sku
                  }
                }
                ... on Magazine {
                  sku
                }
              }
            }
          }
        }
      "#,
    );
    let query_plan = build_query_plan_from_sdl(
        &supergraph,
        document,
        Default::default(),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $title

    fetches
      fetch               after
      [2] products        -
      [7] reviews         [2]
      [4] books           [7]
      [5] products        [7]

    [2] products
      out
        { products { __typename id } }

    [7] reviews   after [2]
      merge
        products.@
      in
        ... on Magazine { __typename id } ... on Tome { __typename id }
      out
        ... on Magazine { reviews { ...a } } ... on Tome { reviews { ...a } }
        fragment a on Review { product { __typename id } }

    [4] books   after [7]
      if
        $title
      merge
        products.@.reviews.@.product|[Tome]
      in
        ... on Tome { __typename id }
      out
        ... on Tome { title }

    [5] products   after [7]
      alias _e0
        if
          $title
        merge
          products.@.reviews.@.product|[Tome]
        in
          ... on Tome { __typename id }
        out
          ... on Tome { sku }
      alias _e1
        merge
          products.@.reviews.@.product|[Magazine]
        in
          ... on Magazine { __typename id }
        out
          ... on Magazine { sku }
    ");

    Ok(())
}
