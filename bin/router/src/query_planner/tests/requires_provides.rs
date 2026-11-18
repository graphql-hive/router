use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn simple_requires_provides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          me {
            reviews {
              id
              author {
                id
                username
              }
              product {
                inStock
              }
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-provides.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] accounts        -
      [3] reviews         [2]
      [4] inventory       [3]

    [2] accounts
      out
        { me { __typename id } }

    [3] reviews   after [2]
      merge
        me
      in
        ... on User { __typename id }
      out
        ... on User { reviews { id author { id username } product { __typename upc } } }

    [4] inventory   after [3]
      merge
        me.reviews.@.product
      in
        ... on Product { __typename upc }
      out
        ... on Product { inStock }
    ");
    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "accounts",
            "operationKind": "query",
            "operation": "{me{__typename id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "me"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "reviews",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{id author{id username} product{__typename upc}}}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "User",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
                    }
                  ]
                }
              ]
            }
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "me"
              },
              {
                "Field": "reviews"
              },
              "@",
              {
                "Field": "product"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "inventory",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{inStock}}}",
              "requires": [
                {
                  "kind": "InlineFragment",
                  "typeCondition": "Product",
                  "selections": [
                    {
                      "kind": "Field",
                      "name": "__typename"
                    },
                    {
                      "kind": "Field",
                      "name": "upc"
                    }
                  ]
                }
              ]
            }
          }
        ]
      }
    }
    "#);
    Ok(())
}

// https://github.com/graphql-hive/router/issues/1455
#[test]
fn provides_fieldset_with_typename_and_requires() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          review {
            author {
              username
              __typename
            }
          }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-requires-typename.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] reviews         -
      [4] secrets         [2]
      [3] reviews         [4]

    [2] reviews
      out
        { review { __typename id } }

    [4] secrets   after [2]
      merge
        review
      in
        ... on Review { __typename id }
      out
        ... on Review { secret }

    [3] reviews   after [4]
      merge
        review
      in
        ... on Review { __typename secret id }
      out
        ... on Review { author { __typename username } }
    ");
    Ok(())
}

/// `label @requires(fields: "name")` on a `User` nested in the entity call for another `User`.
/// The keys for the `names` hop go into that entity call at `other`, not into its parent.
#[test]
fn requires_on_same_type_nested_in_entity_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            other {
              label
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-nested-same-type.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] social          [2]
      [5] names           [3]
      [4] social          [5]

    [2] users
      out
        { user { __typename id } }

    [3] social   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { other { __typename id } }

    [5] names   after [3]
      merge
        user.other
      in
        ... on User { __typename id }
      out
        ... on User { name }

    [4] social   after [5]
      merge
        user.other
      in
        ... on User { __typename name id }
      out
        ... on User { label }
    ");

    Ok(())
}

/// Same as above, with a `@provides` field on the way. `other` is a plain `User`, so `name`
/// still comes from `names`.
#[test]
fn requires_on_same_type_nested_under_provides() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            related {
              other {
                label
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-nested-same-type.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] social          [2]
      [5] names           [3]
      [4] social          [5]

    [2] users
      out
        { user { __typename id } }

    [3] social   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { related { other { __typename id } } }

    [5] names   after [3]
      merge
        user.related.other
      in
        ... on User { __typename id }
      out
        ... on User { name }

    [4] social   after [5]
      merge
        user.related.other
      in
        ... on User { __typename name id }
      out
        ... on User { label }
    ");

    Ok(())
}

/// `User.orders @provides(fields: "item { ... on Book { isbn } }")`, where `item` is a
/// `Book | Movie` union. The copy's `item` only leads to the provided `Book`, while the plain
/// `item` also leads to `Movie`. So the plain route has to stay, or `... on Movie { title }` is
/// dropped from the plan.
#[test]
fn provided_union_keeps_members_that_are_not_provided() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              item {
                ... on Book {
                  isbn
                  stock
                }
                ... on Movie {
                  title
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-union-member.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]
      [4] catalog         [3]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User {
          orders { item { __typename ... on Book { isbn stock } ... on Movie { __typename id } } }
        }

    [4] catalog   after [3]
      merge
        user.orders.@.item|[Movie]
      in
        ... on Movie { __typename id }
      out
        ... on Movie { title }
    ");

    Ok(())
}

/// `User.orders @provides(fields: "sku ...")` and `Store.orders @provides(fields: "weight")` both
/// lead to `Order` in `orders`. Each one only provides its own fields, so under `Store.orders`
/// `sku` has to come from `catalog`.
#[test]
fn provides_does_not_leak_to_another_parent() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          store {
            orders {
              sku
              weight
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-scope.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] catalog         -
      [3] orders          [2]
      [4] catalog         [3]

    [2] catalog
      out
        { store { __typename id } }

    [3] orders   after [2]
      merge
        store
      in
        ... on Store { __typename id }
      out
        ... on Store { orders { __typename id weight } }

    [4] catalog   after [3]
      merge
        store.orders.@
      in
        ... on Order { __typename id }
      out
        ... on Order { sku }
    ");

    Ok(())
}

/// `User.pastOrders` returns `Order` like `User.orders`, but has no `@provides`.
#[test]
fn provides_does_not_apply_to_another_field() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            pastOrders {
              sku
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-scope.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]
      [4] catalog         [3]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { pastOrders { __typename id } }

    [4] catalog   after [3]
      merge
        user.pastOrders.@
      in
        ... on Order { __typename id }
      out
        ... on Order { sku }
    ");

    Ok(())
}

/// `User.orders` copies `ZOrder` before `ZOrder.product @provides(fields: "name")` is handled,
/// because `User` sorts first. The copy still has to get `name` under `product`, `Product` has
/// no key to fetch it from `catalog`.
#[test]
fn provides_reaches_copies_made_earlier() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              sku
              product {
                name
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-copies.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { orders { sku product { name } } }
    ");

    Ok(())
}

/// `User.orders @provides(fields: "sku")`, so `orders` can return `sku` for those orders, and
/// `Order.name @requires(fields: "sku")` lives in `orders` too. So both come from the same
/// fetch: `orders { sku name }`.
///
/// We used to ignore the provided `sku`, fetch it from `catalog`, and then go back to `orders`
/// for `name` - 4 fetches instead of 2.
#[test]
fn requires_provided_after_entity_hop() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              name
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-provided-after-hop.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { orders { sku name } }
    ");

    Ok(())
}

/// Like `requires_provided_after_entity_hop`, but `name` requires `sku weight` and only `sku` is
/// provided. `weight` needs an entity call to `catalog` anyway, so `name` has to go through the
/// usual `@requires` steps.
///
/// The provided `sku` comes from the `orders` fetch under `User.orders`, and only `weight` from
/// `catalog`. It must not end up in an `_entities` call to `orders`: `sku` is `@external` there,
/// so `orders` can only return it under `User.orders`. The `@include` keeps the requirement
/// fetches apart, which is what used to hide this.
#[test]
fn requires_partly_provided_after_entity_hop() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($x: Boolean!) {
          user {
            orders {
              name @include(if: $x)
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/requires-partly-provided-after-hop.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $x: Boolean!

    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]
      [5] catalog         [3]
      [4] orders          [5]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { orders { __typename id sku @include(if: $x) } }
      vars
        $x

    [5] catalog   after [3]
      if
        $x
      merge
        user.orders.@
      in
        ... on Order { __typename id }
      out
        ... on Order { weight }

    [4] orders   after [5]
      if
        $x
      merge
        user.orders.@
      in
        ... on Order { __typename sku weight id }
      out
        ... on Order { name }
    ");

    Ok(())
}

/// `User.orders @provides(fields: "item { ... on Book { isbn } }")`, where `item` is a
/// `Book | Movie` union, and `Order.label @requires(fields: "item { ... on Book { isbn } }")`
/// lives in `orders` too. The provided `isbn` sits behind a type condition, so both still come
/// from the same fetch.
#[test]
fn requires_provided_through_type_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              label
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-union-member.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { orders { item { __typename ... on Book { isbn } } label } }
    ");

    Ok(())
}

/// `mediaLabel @requires(fields: "media { ... on Book { isbn } }")`, where `media` is an
/// interface, and `User.orders` provides `media { ... on Book { isbn } }`. Both come from the
/// same fetch.
#[test]
fn requires_provided_through_interface_condition() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              mediaLabel
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-scope.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { orders { media { __typename ... on Book { isbn } } mediaLabel } }
    ");

    Ok(())
}

/// `User.related: User! @provides(fields: "name")` copies `User` itself, so `related` on the copy
/// has to provide `name` too. The `@requires` of `label` is then met in place at every level.
#[test]
fn provides_on_field_returning_its_own_type() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            label
            related {
              label
              related {
                label
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/provides-copies.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [5] names           [2]
      [3] social          [5]

    [2] users
      out
        { user { __typename id } }

    [5] names   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { name }

    [3] social   after [5]
      merge
        user
      in
        ... on User { __typename id name }
      out
        ... on User { related { name label related { name label } } label }
    ");

    Ok(())
}
