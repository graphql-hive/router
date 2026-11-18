use crate::query_planner::{
    graph::PlannerOverrideContext,
    planner::QueryPlannerOptions,
    tests::testkit::{build_query_plan, build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// https://github.com/graphql-hive/router/issues/1189
///
/// With a variable condition, the fetch is conditional and still contains the selected field.
#[test]
fn issue_1189_variable_skip_is_preserved_in_subgraph_operation() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($v: Boolean!) {
          product @skip(if: $v) {
            price
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $v: Boolean!

    fetches
      fetch               after
      [2] a               -

    [2] a
      if
        !$v
      out
        { product @skip(if: $v) { price } }
      vars
        $v
    ");

    Ok(())
}

#[test]
fn issue_281_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          viewer {
            review {
              ... on AnonymousReview {
                __typename
                product {
                  b
                }
              }
              ... on UserReview {
                __typename
                product {
                  c
                  d
                }
              }
            }
          }
        }

        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/281.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] a               -
      [7] b               [2]
      [4] c               [7]
      [6] d               [7]

    [2] a
      out
        {
          viewer {
            review {
              __typename
              ... on AnonymousReview { __typename product { ...a } }
              ... on UserReview { __typename product { ...a } }
            }
          }
        }
        fragment a on Product { __typename id }

    [7] b   after [2]
      alias _e0
        merge
          viewer.review|[UserReview].product
        in
          ... on Product { __typename id }
        out
          ... on Product { pid }
      alias _e1
        merge
          viewer.review|[AnonymousReview].product
        in
          ... on Product { __typename id }
        out
          ... on Product { b }

    [4] c   after [7]
      merge
        viewer.review|[UserReview].product
      in
        ... on Product { __typename pid }
      out
        ... on Product { c }

    [6] d   after [7]
      merge
        viewer.review|[UserReview].product
      in
        ... on Product { __typename pid }
      out
        ... on Product { d }
    ");

    Ok(())
}

#[test]
fn issue_190_test() -> Result<(), Box<dyn Error>> {
    init_logger();

    // Original version
    let document = parse_operation(
        r#"
        query(
          $included: Boolean!
        ) {
          recommender @include(if: $included) {
            id
            results {
              ...Recommendable_Product
              __typename
            }
            __typename
          }
        }

        fragment Recommendable_Product on Product {
          id
        }
      "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/190.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $included: Boolean!

    fetches
      fetch               after
      [2] recommender     -

    [2] recommender
      if
        $included
      out
        {
          recommender @include(if: $included) {
            __typename
            id
            results { __typename ... on Product { id } }
          }
        }
      vars
        $included
    ");

    // Without __typename version
    let document = parse_operation(
        r#"
        query(
          $included: Boolean!
        ) {
          recommender @include(if: $included) {
            id
            results {
              ...Recommendable_Product
            }
          }
        }

        fragment Recommendable_Product on Product {
          id
        }
      "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/190.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $included: Boolean!

    fetches
      fetch               after
      [2] recommender     -

    [2] recommender
      if
        $included
      out
        { recommender @include(if: $included) { id results { __typename ... on Product { id } } } }
      vars
        $included
    ");

    // Inline fragment version
    let document = parse_operation(
        r#"
        query(
          $included: Boolean!
        ) {
          recommender @include(if: $included) {
            id
            results {
              ... on Product {
                id
              }
            }
          }
        }
      "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/190.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $included: Boolean!

    fetches
      fetch               after
      [2] recommender     -

    [2] recommender
      if
        $included
      out
        { recommender @include(if: $included) { id results { __typename ... on Product { id } } } }
      vars
        $included
    ");

    Ok(())
}

#[test]
fn issue_939_test() -> Result<(), Box<dyn Error>> {
    init_logger();

    let named_fragment_document = parse_operation(
        r#"
        query SingleNode($id: ID!) {
          node(id: $id) {
            ... on MyNode {
              content {
                ... on ITextContent {
                  fragments {
                    contentNode {
                      content {
                        ...ITextContentPreview
                      }
                    }
                  }
                }
              }
            }
          }
        }

        fragment ITextContentPreview on ITextContent {
          id
        }
        "#,
    );
    let named_fragment_plan = build_query_plan_with_defaults(
        "fixture/issues/939.supergraph.graphql",
        named_fragment_document,
    )?;

    let inline_fragment_document = parse_operation(
        r#"
        query SingleNode($id: ID!) {
          node(id: $id) {
            ... on MyNode {
              content {
                ... on ITextContent {
                  fragments {
                    contentNode {
                      content {
                        ... on ITextContent {
                          id
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }
        "#,
    );
    let inline_fragment_plan = build_query_plan_with_defaults(
        "fixture/issues/939.supergraph.graphql",
        inline_fragment_document,
    )?;

    assert_eq!(
        format!("{}", named_fragment_plan),
        format!("{}", inline_fragment_plan)
    );

    insta::assert_snapshot!(format!("{}", inline_fragment_plan), @r"
    variables
      $id: ID!

    fetches
      fetch               after
      [2] content         -

    [2] content
      out
        {
          node(id: $id) {
            __typename
            ... on MyNode {
              content {
                __typename
                ... on TextContent { fragments { ...a } }
                ... on TextGroupContent { fragments { ...a } }
              }
            }
          }
        }
        fragment a on TextContentFragment {
          contentNode {
            content {
              __typename
              ... on TextContent { id }
              ... on TextGroupContent { id }
            }
          }
        }
      vars
        $id
    ");

    Ok(())
}

#[test]
fn experimental_abstract_type_folding_folds_object_fragments_into_interface(
) -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        query SingleNode($id: ID!) {
          node(id: $id) {
            ... on MyNode {
              content {
                ... on TextContent {
                  id
                }
                ... on TextGroupContent {
                  id
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/939.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $id: ID!

    fetches
      fetch               after
      [2] content         -

    [2] content
      out
        {
          node(id: $id) {
            __typename
            ... on MyNode {
              content { __typename ... on TextContent { id } ... on TextGroupContent { id } }
            }
          }
        }
      vars
        $id
    ");

    let document = parse_operation(
        r#"
        query SingleNode($id: ID!) {
          node(id: $id) {
            ... on MyNode {
              content {
                ... on TextContent {
                  id
                }
                ... on TextGroupContent {
                  id
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan(
        "fixture/issues/939.supergraph.graphql",
        document,
        Default::default(),
        QueryPlannerOptions {
            experimental_abstract_type_folding: true,
            ..Default::default()
        },
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $id: ID!

    fetches
      fetch               after
      [2] content         -

    [2] content
      out
        {
          node(id: $id) {
            __typename
            ... on MyNode { content { __typename ... on ITextContent { id } } }
          }
        }
      vars
        $id
    ");

    Ok(())
}

#[test]
fn issue_965_test() -> Result<(), Box<dyn Error>> {
    init_logger();

    let abstract_named_fragment_document = parse_operation(
        r#"
        query {
          account(id: "a1") {
            ...Test
          }
        }

        fragment Test on Node {
          ... on Node {
            id
          }
          ... on Account {
            username
          }
        }
        "#,
    );
    let abstract_named_fragment_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        abstract_named_fragment_document,
    )?;

    let concrete_named_fragment_document = parse_operation(
        r#"
        query {
          account(id: "a1") {
            ...Test
          }
        }

        fragment Test on Account {
          ... on Node {
            id
          }
          username
        }
        "#,
    );
    let concrete_named_fragment_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        concrete_named_fragment_document,
    )?;

    insta::assert_snapshot!(format!("{}", concrete_named_fragment_plan), @r#"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { account(id: "a1") { id username } }
    "#);

    insta::assert_snapshot!(format!("{}", abstract_named_fragment_plan), @r#"
    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        { account(id: "a1") { id username } }
    "#);

    Ok(())
}

#[test]
fn issue_965_mixed_nested_fragments_with_directives_test() -> Result<(), Box<dyn Error>> {
    init_logger();

    let abstract_named_fragment_document = parse_operation(
        r#"
        query($outer: Boolean!, $inner: Boolean!, $skip: Boolean!) {
          account(id: "a1") {
            ...Test
          }
        }

        fragment Test on Node {
          ... on Node @include(if: $outer) {
            ...Inner @include(if: $inner)
          }
          ... on Account @skip(if: $skip) {
            username
          }
        }

        fragment Inner on Node {
          ... {
            id
          }
        }
        "#,
    );
    let abstract_named_fragment_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        abstract_named_fragment_document,
    )?;

    let concrete_named_fragment_document = parse_operation(
        r#"
        query($outer: Boolean!, $inner: Boolean!, $skip: Boolean!) {
          account(id: "a1") {
            ...Test
          }
        }

        fragment Test on Account {
          ... on Account @include(if: $outer) {
            ...Inner @include(if: $inner)
          }
          ... on Account @skip(if: $skip) {
            username
          }
        }

        fragment Inner on Account {
          id
        }
        "#,
    );
    let concrete_named_fragment_plan = build_query_plan_with_defaults(
        "fixture/tests/corrupted-supergraph-node-id.supergraph.graphql",
        concrete_named_fragment_document,
    )?;

    insta::assert_snapshot!(format!("{}", concrete_named_fragment_plan), @r#"
    variables
      $inner: Boolean!, $outer: Boolean!, $skip: Boolean!

    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        {
          account(id: "a1") {
            ... on Account @include(if: $outer) { ... on Account @include(if: $inner) { id } }
            ... on Account @skip(if: $skip) { username }
          }
        }
      vars
        $inner, $outer, $skip
    "#);

    insta::assert_snapshot!(format!("{}", abstract_named_fragment_plan), @r#"
    variables
      $inner: Boolean!, $outer: Boolean!, $skip: Boolean!

    fetches
      fetch               after
      [2] a               -

    [2] a
      out
        {
          account(id: "a1") {
            ... on Account @include(if: $outer) { ... on Account @include(if: $inner) { id } }
            ... on Account @skip(if: $skip) { username }
          }
        }
      vars
        $inner, $outer, $skip
    "#);

    Ok(())
}

#[test]
fn issue_interface_object_typename() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        {
          me {
            __typename
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/issues/infinite-typename-interfaceobject.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] s3              -

    [2] s3
      out
        { me { __typename } }
    ");

    Ok(())
}

#[test]
fn recursion_bomb_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          list {
            items {
              value
            }
          }
        }
        "#,
    );
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let ok = build_query_plan_with_defaults(
            "fixture/issues/recursion-bomb.supergraph.graphql",
            document,
        )
        .is_ok();

        let _ = tx.send(ok);
    });

    let ok = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("query planner timed out");

    assert!(ok);

    Ok(())
}

#[test]
fn requires_circular_keys_test() -> Result<(), Box<dyn Error>> {
    init_logger();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let queries = [
            r#"
            {
              list {
                items {
                  value
                  valueRequiresObjectFragment
                  valueRequiresInterfaceFragment
                  valueRequiresInlineFragment
                  valueNestedField
                }
              }
            }
            "#,
            r#"
            {
              list {
                taggedItems {
                  ... on Item {
                    valueInterfaceKey
                  }
                }
              }
            }
            "#,
        ];

        let result = queries.iter().try_for_each(|query| {
            let document = parse_operation(query);
            build_query_plan_with_defaults(
                "fixture/issues/requires-circular-keys.supergraph.graphql",
                document,
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        });

        let _ = tx.send(result);
    });

    let result = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("query planner timed out");

    assert!(result.is_ok(), "{}", result.unwrap_err());

    Ok(())
}

#[test]
fn requires_self_dependency_false_positive() -> Result<(), Box<dyn Error>> {
    init_logger();

    let document = parse_operation(
        r#"
        {
          books {
            edition {
              book {
                catalogId
              }
            }
            isRecommended
          }
        }
        "#,
    );

    let plan = build_query_plan_with_defaults(
        "fixture/issues/requires-self-dependency-false-positive.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", plan), @r"
    fetches
      fetch               after
      [2] library         -
      [4] recommender     [2]

    [2] library
      out
        { books { __typename edition { book { catalogId } isbn } } }

    [4] recommender   after [2]
      merge
        books.@
      in
        ... on BookListing { __typename edition { book { catalogId } isbn } }
      out
        ... on BookListing { isRecommended }
    ");

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1539
///
/// `Order.name @requires(fields: "sku")`, where the `Order` came out of an `_entities` fetch.
/// That same fetch should also ask for `sku`, instead of us going back to `orders` for it.
#[test]
fn issue_1539_requires_after_entity_hop() -> Result<(), Box<dyn Error>> {
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
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1539.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]
      [5] catalog         [3]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { orders { __typename id sku } }

    [5] catalog   after [3]
      merge
        user.orders.@
      in
        ... on Order { __typename sku id }
      out
        ... on Order { name }
    ");

    Ok(())
}

/// Same as `issue_1539_requires_after_entity_hop`, but the query already asks for `sku`.
#[test]
fn issue_1539_requires_after_entity_hop_with_explicit_requirement() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          user {
            orders {
              sku
              name
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1539.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]
      [5] catalog         [3]

    [2] users
      out
        { user { __typename id } }

    [3] orders   after [2]
      merge
        user
      in
        ... on User { __typename id }
      out
        ... on User { orders { __typename sku id } }

    [5] catalog   after [3]
      merge
        user.orders.@
      in
        ... on Order { __typename sku id }
      out
        ... on Order { name }
    ");

    Ok(())
}

/// The other half of `issue_1539_requires_after_entity_hop`.
/// Here the parent comes straight from a root field, so there is no entity hop,
/// and `sku` was always added to the fetch that resolves the parent.
#[test]
fn issue_1539_requires_on_root_parent() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          order {
            name
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1539.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] orders          -
      [4] catalog         [2]

    [2] orders
      out
        { order { __typename id sku } }

    [4] catalog   after [2]
      merge
        order
      in
        ... on Order { __typename sku id }
      out
        ... on Order { name }
    ");

    Ok(())
}

/// The parent entity call and the nested one can be the same type (`Order.related: Order`).
/// This still has to work, and we must not copy the nested call's keys up into the parent,
/// as they belong to a different path.
#[test]
fn issue_1539_requires_on_self_referential_entity() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          order {
            related {
              name
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/issues/1539-self-referential.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] users           -
      [3] orders          [2]
      [5] catalog         [3]

    [2] users
      out
        { order { __typename id } }

    [3] orders   after [2]
      merge
        order
      in
        ... on Order { __typename id }
      out
        ... on Order { related { __typename id sku } }

    [5] catalog   after [3]
      merge
        order.related
      in
        ... on Order { __typename sku id }
      out
        ... on Order { name }
    ");

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1308
/// Requirements crossing an entity interface must preserve their concrete type conditions
/// after an additional entity hop.
#[test]
fn issue_1308_requires_through_entity_interface_after_entity_hop() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          cage {
            listings {
              rank
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1308.supergraph.graphql", document)?;
    let query_plan = query_plan.to_string();

    assert!(query_plan.contains("tricks"));
    assert!(query_plan.contains("whiskers"));
    assert!(query_plan.contains("rank"));

    Ok(())
}

/// Reduced from generated fixture #1308, seed 0. The field-level condition reaches a different
/// failure path than the unconditional issue query: planning loses the `Animal` definition while
/// resolving the concrete-type requirements for `rank`.
/// The `search` call for `pet` only needs the `search` call above it, so it goes into that one,
/// under `... on Listing @include(if: $includeRank)`.
#[test]
fn issue_1308_conditional_rank_keeps_concrete_requires_types() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($includeRank: Boolean!) {
          cage {
            listings {
              rank @include(if: $includeRank)
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1308.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r"
    variables
      $includeRank: Boolean!

    fetches
      fetch               after
      [2] catalog         -
      [3] search          [2]
      [8] catalog         [3]
      [5] ranking         [8]

    [2] catalog
      out
        { cage { __typename id } }

    [3] search   after [2]
      merge
        cage
      in
        ... on Cage { __typename id }
      out
        ... on Cage {
          listings { __typename id ... on Listing @include(if: $includeRank) { pet { __typename id } } }
        }
      vars
        $includeRank

    [8] catalog   after [3]
      if
        $includeRank
      merge
        cage.listings.@.pet
      in
        ... on Animal { __typename ... on Cat { id } ... on Dog { id } }
      out
        ... on Animal { __typename ... on Cat { whiskers } ... on Dog { tricks } }

    [5] ranking   after [8]
      if
        $includeRank
      merge
        cage.listings.@
      in
        ... on Listing {
          __typename
          pet { __typename ... on Dog { tricks } ... on Cat { whiskers } }
          id
        }
      out
        ... on Listing { rank }
    ");

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1309
/// Requirements through an entity interface must retain concrete-type field conditions.
/// `whiskers` and `tricks` can each come through `Animal`'s key or their own type's key, at
/// the same cost. Both go one way, so `catalog` is called once.
#[test]
fn issue_1309_requires_through_entity_interface() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          listings {
            rank
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1309.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] search          -
      [7] catalog         [2]
      [4] ranking         [7]

    [2] search
      out
        { listings { __typename id pet { __typename id } } }

    [7] catalog   after [2]
      merge
        listings.@.pet
      in
        ... on Animal { __typename ... on Cat { id } ... on Dog { id } }
      out
        ... on Animal { __typename ... on Cat { whiskers } ... on Dog { tricks } }

    [4] ranking   after [7]
      merge
        listings.@
      in
        ... on Listing {
          __typename
          pet { __typename ... on Dog { tricks } ... on Cat { whiskers } }
          id
        }
      out
        ... on Listing { rank }
    ");

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1310
/// A nested `@requires` can depend on fields fetched from separate subgraphs.
#[test]
fn issue_1310_nested_requires_with_multiple_providers() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          pet {
            checkup {
              grade
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1310.supergraph.graphql", document)?;
    let query_plan = query_plan.to_string();

    for expected in ["age", "weight", "fee", "grade"] {
        assert!(
            query_plan.contains(expected),
            "missing {expected}: {query_plan}"
        );
    }

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1311
/// Same field with different arguments under separate type conditions must not panic or lose
/// either requested value while planning.
#[test]
fn issue_1311_different_arguments_under_type_conditions() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          storefront {
            departments {
              ... on Aquatics {
                photo {
                  thumbnail(width: 100)
                }
              }
              ... on Reptiles {
                photo {
                  thumbnail(width: 200)
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1311.supergraph.graphql", document)?;
    let query_plan = query_plan.to_string();

    assert!(query_plan.contains("thumbnail(width: 100)"));
    assert!(query_plan.contains("thumbnail(width: 200)"));

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1647
/// `record` and `search` both sit under `@include(if: $withDetails)`, in one `accounts` fetch
/// that runs either way. `invoices` sends `record` on to `billing`, and that call is under the
/// condition. `search` has to keep its `@include`, or `accounts` resolves it when
/// `$withDetails` is false.
#[test]
fn issue_1647_include_on_field_after_conditional_entity_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($withDetails: Boolean!, $id: ID! = "", $term: String! = "") {
          record(id: $id) @include(if: $withDetails) {
            ... on User {
              id
              email
              invoices
            }
          }
          listed: catalog {
            search(term: $term, kind: "item", source: "store") @include(if: $withDetails) {
              ... on Item {
                id
                label
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1647.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    variables
      $id: ID! = "", $term: String! = "", $withDetails: Boolean!

    fetches
      fetch               after
      [2] accounts        -
      [3] billing         [2]

    [2] accounts
      out
        {
          record(id: $id) @include(if: $withDetails) { __typename ... on User { __typename id email } }
          listed: catalog {
            search(kind: "item", source: "store", term: $term) @include(if: $withDetails) {
              __typename
              ... on Item { id label }
            }
          }
        }
      vars
        $id, $term, $withDetails

    [3] billing   after [2]
      if
        $withDetails
      merge
        record|[User]
      in
        ... on User { __typename id }
      out
        ... on User { invoices }
    "#);

    Ok(())
}

/// Same as above, with the condition on a `... on Query` fragment instead of on `search`.
/// The fragment has to keep its `@include`, or `accounts` resolves `listed` when
/// `$withDetails` is false.
#[test]
fn issue_1647_include_on_fragment_after_conditional_entity_call() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query ($withDetails: Boolean!, $id: ID! = "", $term: String! = "") {
          record(id: $id) @include(if: $withDetails) {
            ... on User {
              id
              email
              invoices
            }
          }
          ... on Query @include(if: $withDetails) {
            listed: catalog {
              search(term: $term, kind: "item", source: "store") {
                ... on Item {
                  id
                  label
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1647.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    variables
      $id: ID! = "", $term: String! = "", $withDetails: Boolean!

    fetches
      fetch               after
      [2] accounts        -
      [3] billing         [2]

    [2] accounts
      if
        $withDetails
      out
        {
          record(id: $id) @include(if: $withDetails) { __typename ... on User { __typename id email } }
          ... on Query @include(if: $withDetails) {
            listed: catalog {
              search(kind: "item", source: "store", term: $term) { __typename ... on Item { id label } }
            }
          }
        }
      vars
        $id, $term, $withDetails

    [3] billing   after [2]
      if
        $withDetails
      merge
        record|[User]
      in
        ... on User { __typename id }
      out
        ... on User { invoices }
    "#);

    Ok(())
}

/// `nested-field-key` resolves `Item` by `group { tag }`, `source` by `group { id }`. The call
/// for `valueNestedField` goes to `nested-field-key`, so it sends `group { tag }`, not the key
/// of another subgraph.
#[test]
fn requires_sends_the_key_of_its_own_subgraph() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          list {
            items {
              valueNestedField
            }
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/issues/requires-circular-keys.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch                 after
      [2] source            -
      [4] nested-field-key  [2]

    [2] source
      out
        { list { items { __typename group { tag id } tag { id } } } }

    [4] nested-field-key   after [2]
      merge
        list.items.@
      in
        ... on Item { __typename tag { id } group { tag } }
      out
        ... on Item { valueNestedField }
    ");

    Ok(())
}

/// `head` resolves `Item` by `code`, `price` requires `weight` from `other`, which takes `id`.
/// `source` holds the items and has `id` too, so it fetches it for the jump to `other`. No call
/// to `head` just for `id`.
#[test]
fn requires_jump_key_comes_from_the_subgraph_holding_the_objects() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          items {
            price
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/issues/requires-jump-to-other-subgraph.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] source          -
      [6] other           [2]
      [4] head            [6]

    [2] source
      out
        { items { __typename code id } }

    [6] other   after [2]
      merge
        items.@
      in
        ... on Item { __typename id }
      out
        ... on Item { weight }

    [4] head   after [6]
      merge
        items.@
      in
        ... on Item { __typename weight code }
      out
        ... on Item { price }
    ");

    Ok(())
}

/// Same as above, but only `head` and `other` have `id`. `source` can't fetch it, so `head` is
/// called for `id` first.
#[test]
fn requires_jump_key_only_in_the_requiring_subgraph() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        {
          items {
            price
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/issues/requires-jump-key-not-held.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] source          -
      [5] head            [2]
      [6] other           [5]
      [4] head            [6]

    [2] source
      out
        { items { __typename code } }

    [5] head   after [2]
      merge
        items.@
      in
        ... on Item { __typename code }
      out
        ... on Item { __typename id }

    [6] other   after [5]
      merge
        items.@
      in
        ... on Item { __typename id }
      out
        ... on Item { weight }

    [4] head   after [6]
      merge
        items.@
      in
        ... on Item { __typename weight code }
      out
        ... on Item { price }
    ");

    Ok(())
}

/// Like above, but `source` only has `id` on `items @provides(fields: "id")`. Its fetch goes
/// through that field, so it fetches `id` for the jump to `other`.
#[test]
fn requires_jump_key_provided_to_the_subgraph_holding_the_objects() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation("{ items { price } }");
    let query_plan = build_query_plan_with_defaults(
        "fixture/issues/requires-jump-key-provided.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] source          -
      [6] other           [2]
      [4] head            [6]

    [2] source
      out
        { items { __typename code id } }

    [6] other   after [2]
      merge
        items.@
      in
        ... on Item { __typename id }
      out
        ... on Item { weight }

    [4] head   after [6]
      merge
        items.@
      in
        ... on Item { __typename weight code }
      out
        ... on Item { price }
    ");

    Ok(())
}

/// `source` takes `id` from `legacy` behind the `id_in_source` label. With the flag on, `source`
/// fetches `id` for the jump to `other`. With it off, `source` doesn't serve `id`, so `head` is
/// called for it.
#[test]
fn requires_jump_key_follows_progressive_override() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = |override_context| {
        build_query_plan(
            "fixture/issues/requires-jump-key-overridden.supergraph.graphql",
            parse_operation("{ items { price } }"),
            override_context,
            Default::default(),
        )
    };

    let query_plan = plan(PlannerOverrideContext::from_flag("id_in_source".into()))?;
    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] source          -
      [6] other           [2]
      [4] head            [6]

    [2] source
      out
        { items { __typename code id } }

    [6] other   after [2]
      merge
        items.@
      in
        ... on Item { __typename id }
      out
        ... on Item { weight }

    [4] head   after [6]
      merge
        items.@
      in
        ... on Item { __typename weight code }
      out
        ... on Item { price }
    ");

    let query_plan = plan(PlannerOverrideContext::from_flag("different_flag".into()))?;
    insta::assert_snapshot!(format!("{}", query_plan), @r"
    fetches
      fetch               after
      [2] source          -
      [5] head            [2]
      [6] other           [5]
      [4] head            [6]

    [2] source
      out
        { items { __typename code } }

    [5] head   after [2]
      merge
        items.@
      in
        ... on Item { __typename code }
      out
        ... on Item { __typename id }

    [6] other   after [5]
      merge
        items.@
      in
        ... on Item { __typename id }
      out
        ... on Item { weight }

    [4] head   after [6]
      merge
        items.@
      in
        ... on Item { __typename weight code }
      out
        ... on Item { price }
    ");

    Ok(())
}
