use crate::query_planner::{
    planner::QueryPlannerOptions,
    tests::testkit::{build_query_plan, build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          {
            viewer {
              review {
                __typename
                ... on AnonymousReview {
                  __typename
                  product {
                    ...a
                  }
                }
                ... on UserReview {
                  __typename
                  product {
                    ...a
                  }
                }
              }
            }
          }
          fragment a on Product {
            __typename
            id
          }
        },
        Flatten(path: "viewer.review.product") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                id
              }
            } =>
            {
              ... on Product {
                pid
                b
              }
            }
          },
        },
        Parallel {
          Flatten(path: "viewer.review|[UserReview].product") {
            Fetch(service: "d") {
              {
                ... on Product {
                  __typename
                  pid
                }
              } =>
              {
                ... on Product {
                  d
                }
              }
            },
          },
          Flatten(path: "viewer.review|[UserReview].product") {
            Fetch(service: "c") {
              {
                ... on Product {
                  __typename
                  pid
                }
              } =>
              {
                ... on Product {
                  c
                }
              }
            },
          },
        },
      },
    },
    "#);

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
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Include(if: $included) {
        Fetch(service: "recommender") {
          {
            recommender {
              id
              results {
                __typename
                ... on Product {
                  id
                }
              }
              __typename
            }
          }
        },
      },
    },
    "#);

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
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Include(if: $included) {
        Fetch(service: "recommender") {
          {
            recommender {
              id
              results {
                __typename
                ... on Product {
                  id
                }
              }
            }
          }
        },
      },
    },
    "#);

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
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Include(if: $included) {
        Fetch(service: "recommender") {
          {
            recommender {
              id
              results {
                __typename
                ... on Product {
                  id
                }
              }
            }
          }
        },
      },
    },
    "#);

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

    insta::assert_snapshot!(format!("{}", inline_fragment_plan), @r#"
    QueryPlan {
      Fetch(service: "content") {
        query ($id:ID!) {
          node(id: $id) {
            __typename
            ... on MyNode {
              content {
                __typename
                ... on TextContent {
                  fragments {
                    ...a
                  }
                }
                ... on TextGroupContent {
                  fragments {
                    ...a
                  }
                }
              }
            }
          }
        }
        fragment a on TextContentFragment {
          contentNode {
            content {
              __typename
              ... on TextContent {
                id
              }
              ... on TextGroupContent {
                id
              }
            }
          }
        }
      },
    },
    "#);

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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "content") {
        query ($id:ID!) {
          node(id: $id) {
            __typename
            ... on MyNode {
              content {
                __typename
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
      },
    },
    "#);

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
        },
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "content") {
        query ($id:ID!) {
          node(id: $id) {
            __typename
            ... on MyNode {
              content {
                __typename
                ... on ITextContent {
                  id
                }
              }
            }
          }
        }
      },
    },
    "#);

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
    QueryPlan {
      Fetch(service: "a") {
        {
          account(id: "a1") {
            id
            username
          }
        }
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", abstract_named_fragment_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        {
          account(id: "a1") {
            id
            username
          }
        }
      },
    },
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
    QueryPlan {
      Fetch(service: "a") {
        query ($inner:Boolean!,$outer:Boolean!,$skip:Boolean!) {
          account(id: "a1") {
            ... on Account @include(if: $outer) {
              ... on Account @include(if: $inner) {
                id
              }
            }
            ... on Account @skip(if: $skip) {
              username
            }
          }
        }
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", abstract_named_fragment_plan), @r#"
    QueryPlan {
      Fetch(service: "a") {
        query ($inner:Boolean!,$outer:Boolean!,$skip:Boolean!) {
          account(id: "a1") {
            ... on Account @include(if: $outer) {
              ... on Account @include(if: $inner) {
                id
              }
            }
            ... on Account @skip(if: $skip) {
              username
            }
          }
        }
      },
    },
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "s3") {
        {
          me {
            __typename
          }
        }
      },
    },
    "#);

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

    insta::assert_snapshot!(format!("{}", plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "library") {
          {
            books {
              __typename
              edition {
                book {
                  catalogId
                }
                isbn
              }
            }
          }
        },
        Flatten(path: "books.@") {
          Fetch(service: "recommender") {
            {
              ... on BookListing {
                __typename
                edition {
                  book {
                    catalogId
                  }
                  isbn
                }
              }
            } =>
            {
              ... on BookListing {
                isRecommended
              }
            }
          },
        },
      },
    },
    "#);

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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "users") {
          {
            user {
              __typename
              id
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "orders") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                orders {
                  __typename
                  id
                  sku
                }
              }
            }
          },
        },
        Flatten(path: "user.orders.@") {
          Fetch(service: "catalog") {
            {
              ... on Order {
                __typename
                sku
                id
              }
            } =>
            {
              ... on Order {
                name
              }
            }
          },
        },
      },
    },
    "#);

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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "users") {
          {
            user {
              __typename
              id
            }
          }
        },
        Flatten(path: "user") {
          Fetch(service: "orders") {
            {
              ... on User {
                __typename
                id
              }
            } =>
            {
              ... on User {
                orders {
                  __typename
                  sku
                  id
                }
              }
            }
          },
        },
        Flatten(path: "user.orders.@") {
          Fetch(service: "catalog") {
            {
              ... on Order {
                __typename
                sku
                id
              }
            } =>
            {
              ... on Order {
                name
              }
            }
          },
        },
      },
    },
    "#);

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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "orders") {
          {
            order {
              __typename
              id
              sku
            }
          }
        },
        Flatten(path: "order") {
          Fetch(service: "catalog") {
            {
              ... on Order {
                __typename
                sku
                id
              }
            } =>
            {
              ... on Order {
                name
              }
            }
          },
        },
      },
    },
    "#);

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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "users") {
          {
            order {
              __typename
              id
            }
          }
        },
        Flatten(path: "order") {
          Fetch(service: "orders") {
            {
              ... on Order {
                __typename
                id
              }
            } =>
            {
              ... on Order {
                related {
                  __typename
                  id
                  sku
                }
              }
            }
          },
        },
        Flatten(path: "order.related") {
          Fetch(service: "catalog") {
            {
              ... on Order {
                __typename
                sku
                id
              }
            } =>
            {
              ... on Order {
                name
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1311
///
/// `thumbnail` is asked for with a different `width` under each type condition. The walker
/// makes one `media` fetch per branch, and they used to get batched into one, where the merge
/// panicked on the argument conflict.
///
/// Aliasing one side doesn't help. It's an `_entities` fetch of photos, nothing in its
/// response says which photo came from `Aquatics`, so the alias can't be renamed back for the
/// right ones. Each branch keeps its own entity call, with its own path.
#[test]
fn issue_1311_same_field_with_different_arguments_under_different_types(
) -> Result<(), Box<dyn Error>> {
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "catalog") {
          {
            storefront {
              departments {
                __typename
                ... on Aquatics {
                  photo {
                    ...a
                  }
                }
                ... on Reptiles {
                  photo {
                    ...a
                  }
                }
              }
            }
          }
          fragment a on Photo {
            __typename
            id
          }
        },
        BatchFetch(service: "media") {
          {
            _e0 {
              paths: [
                "storefront.departments.@|[Reptiles].photo"
              ]
              {
                ... on Photo {
                  __typename
                  id
                }
              }
            }
            _e1 {
              paths: [
                "storefront.departments.@|[Aquatics].photo"
              ]
              {
                ... on Photo {
                  __typename
                  id
                }
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Photo {
                thumbnail(width: 200)
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on Photo {
                thumbnail(width: 100)
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Same as `issue_1311_same_field_with_different_arguments_under_different_types`, but each
/// `thumbnail` has its own `@include`. The merge puts each one under its own
/// `... on Photo @include(...)`, and we used to miss the conflict there, so both ended up in
/// one `Photo` selection. That's not a valid operation, and with both variables true every
/// photo would get both widths. Different conditions don't make two fields exclusive for
/// GraphQL validation.
#[test]
fn issue_1311_conflict_under_different_include_conditions() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query($a: Boolean!, $b: Boolean!) {
          storefront {
            departments {
              ... on Aquatics {
                photo {
                  thumbnail(width: 100) @include(if: $a)
                }
              }
              ... on Reptiles {
                photo {
                  thumbnail(width: 200) @include(if: $b)
                }
              }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/issues/1311.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "catalog") {
          {
            storefront {
              departments {
                __typename
                ... on Aquatics {
                  photo {
                    ...a
                  }
                }
                ... on Reptiles {
                  photo {
                    ...a
                  }
                }
              }
            }
          }
          fragment a on Photo {
            __typename
            id
          }
        },
        Parallel {
          Include(if: $b) {
            Flatten(path: "storefront.departments.@|[Reptiles].photo") {
              Fetch(service: "media") {
                {
                  ... on Photo {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Photo {
                    thumbnail(width: 200)
                  }
                }
              },
            },
          },
          Include(if: $a) {
            Flatten(path: "storefront.departments.@|[Aquatics].photo") {
              Fetch(service: "media") {
                {
                  ... on Photo {
                    __typename
                    id
                  }
                } =>
                {
                  ... on Photo {
                    thumbnail(width: 100)
                  }
                }
              },
            },
          },
        },
      },
    },
    "#);

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1310
///
/// `Checkup.grade @requires(fields: "fee")` sits inside `Pet.checkup @requires(fields: "weight age")`,
/// and `weight` and `age` come from two different subgraphs. The step with `checkup` waits on
/// both of them, and `process_requires_field_edge` used to fail with `NonSingleParent`.
#[test]
fn issue_1310_requires_inside_requires_with_two_providers() -> Result<(), Box<dyn Error>> {
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "clinic") {
          {
            pet {
              __typename
              id
            }
          }
        },
        Parallel {
          Flatten(path: "pet") {
            Fetch(service: "records") {
              {
                ... on Pet {
                  __typename
                  id
                }
              } =>
              {
                ... on Pet {
                  weight
                }
              }
            },
          },
          Flatten(path: "pet") {
            Fetch(service: "profiles") {
              {
                ... on Pet {
                  __typename
                  id
                }
              } =>
              {
                ... on Pet {
                  age
                }
              }
            },
          },
        },
        Flatten(path: "pet") {
          Fetch(service: "clinic") {
            {
              ... on Pet {
                __typename
                weight
                age
                id
              }
            } =>
            {
              ... on Pet {
                checkup {
                  __typename
                  id
                }
              }
            }
          },
        },
        Flatten(path: "pet.checkup") {
          Fetch(service: "records") {
            {
              ... on Checkup {
                __typename
                id
              }
            } =>
            {
              ... on Checkup {
                fee
              }
            }
          },
        },
        Flatten(path: "pet.checkup") {
          Fetch(service: "clinic") {
            {
              ... on Checkup {
                __typename
                fee
                id
              }
            } =>
            {
              ... on Checkup {
                grade
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1309
///
/// `Listing.rank @requires` reads `tricks` and `whiskers` through the `Animal` entity
/// interface. The `Dog` and `Cat` fetches get merged into the `Animal` one, and their fields
/// used to lose the type conditions, so `whiskers` ended up right on `Animal`.
///
/// The issue's `ranking` subgraph is missing `Bird`, which Apollo refuses to compose, so the
/// fixture adds it.
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "search") {
          {
            listings {
              __typename
              id
              pet {
                __typename
                id
              }
            }
          }
        },
        Flatten(path: "listings.@.pet") {
          Fetch(service: "catalog") {
            {
              ... on Animal {
                __typename
                id
              }
            } =>
            {
              ... on Animal {
                __typename
                ... on Cat {
                  whiskers
                }
                ... on Dog {
                  tricks
                }
              }
            }
          },
        },
        Flatten(path: "listings.@") {
          Fetch(service: "ranking") {
            {
              ... on Listing {
                __typename
                pet {
                  __typename
                  ... on Dog {
                    tricks
                  }
                  ... on Cat {
                    whiskers
                  }
                }
                id
              }
            } =>
            {
              ... on Listing {
                rank
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1308
///
/// Same schema as `issue_1309_requires_through_entity_interface`, one entity hop deeper.
/// It used to fail with `UnexpectedMissingDefinition("Animal")` or with the #1309 error,
/// depending on which way the sibling fetches got merged. The other direction is covered by
/// `multi_type_step_does_not_absorb_step_of_another_type`.
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "catalog") {
          {
            cage {
              __typename
              id
            }
          }
        },
        Flatten(path: "cage") {
          Fetch(service: "search") {
            {
              ... on Cage {
                __typename
                id
              }
            } =>
            {
              ... on Cage {
                listings {
                  __typename
                  id
                  pet {
                    __typename
                    id
                  }
                }
              }
            }
          },
        },
        Flatten(path: "cage.listings.@.pet") {
          Fetch(service: "catalog") {
            {
              ... on Animal {
                __typename
                id
              }
            } =>
            {
              ... on Animal {
                __typename
                ... on Cat {
                  whiskers
                }
                ... on Dog {
                  tricks
                }
              }
            }
          },
        },
        Flatten(path: "cage.listings.@") {
          Fetch(service: "ranking") {
            {
              ... on Listing {
                __typename
                pet {
                  __typename
                  ... on Dog {
                    tricks
                  }
                  ... on Cat {
                    whiskers
                  }
                }
                id
              }
            } =>
            {
              ... on Listing {
                rank
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1189
///
/// `@skip(if: true)` on the only root field leaves nothing to plan. That's an empty plan,
/// not an error, and the router should answer `{"data": {}}`.
#[test]
fn issue_1189_nothing_to_plan_after_static_skip() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        query {
          product @skip(if: true) {
            price
          }
        }
        "#,
    );
    let query_plan = build_query_plan_with_defaults(
        "fixture/tests/simple-include-skip.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      None,
    },
    "#);

    Ok(())
}

/// https://github.com/graphql-hive/router/issues/1189
///
/// Same as `issue_1189_nothing_to_plan_after_static_skip`, but through a variable. The whole
/// fetch goes under `Skip`, so with `$v: true` we don't send `a` a query with nothing in it.
#[test]
fn issue_1189_nothing_to_fetch_after_variable_skip() -> Result<(), Box<dyn Error>> {
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

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Skip(if: $v) {
        Fetch(service: "a") {
          {
            product {
              price
            }
          }
        },
      },
    },
    "#);

    Ok(())
}
