use crate::query_planner::{
    tests::testkit::{build_query_plan_from_sdl, build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

#[test]
fn mutations() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          addProduct(input: { name: "new", price: 599.99 }) {
            name
            price
            isExpensive
            isAvailable
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            addProduct(input: {name: "new", price: 599.99}) {
              __typename
              name
              price
              id
            }
          }
        },
        Flatten(path: "addProduct") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
                isAvailable
              }
            }
          },
        },
      },
    },
    "#);

    insta::assert_snapshot!(format!("{}", sonic_rs::to_string_pretty(&query_plan).unwrap_or_default()), @r#"
    {
      "kind": "QueryPlan",
      "node": {
        "kind": "Sequence",
        "nodes": [
          {
            "kind": "Fetch",
            "serviceName": "a",
            "operationKind": "mutation",
            "operation": "mutation{addProduct(input: {name: \"new\", price: 599.99}){__typename name price id}}"
          },
          {
            "kind": "Flatten",
            "path": [
              {
                "Field": "addProduct"
              }
            ],
            "node": {
              "kind": "Fetch",
              "serviceName": "b",
              "operationKind": "query",
              "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{isExpensive isAvailable}}}",
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
                      "name": "price"
                    },
                    {
                      "kind": "Field",
                      "name": "id"
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

#[test]
fn many_fields_two_same_graph() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          five: add(num: 5)
          ten: multiply(by: 2)
          twelve: add(num: 2)
          final: delete
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          mutation {
            five: add(num: 5)
          }
        },
        Fetch(service: "a") {
          mutation {
            ten: multiply(by: 2)
          }
        },
        Fetch(service: "c") {
          mutation {
            twelve: add(num: 2)
          }
        },
        Fetch(service: "b") {
          mutation {
            final: delete
          }
        },
      },
    },
    "#);

    let document = parse_operation(
        r#"
        mutation {
          five: add(num: 5)
          seven: add(num: 2)
          fourteen: multiply(by: 2)
          sixteen: add(num: 2)
          final: delete
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          mutation {
            five: add(num: 5)
            seven: add(num: 2)
          }
        },
        Fetch(service: "a") {
          mutation {
            fourteen: multiply(by: 2)
          }
        },
        Fetch(service: "c") {
          mutation {
            sixteen: add(num: 2)
          }
        },
        Fetch(service: "b") {
          mutation {
            final: delete
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Three mutations in a row on one subgraph go in one fetch. After the first two are merged,
/// the third one has to still count as next to them.
#[test]
fn three_fields_in_a_row_same_graph() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          one: add(num: 1)
          two: add(num: 2)
          three: add(num: 3)
          final: delete
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          mutation {
            one: add(num: 1)
            two: add(num: 2)
            three: add(num: 3)
          }
        },
        Fetch(service: "b") {
          mutation {
            final: delete
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `count` starts only once `create` is done, and that includes `isExpensive` from `b`.
#[test]
fn next_field_waits_for_entity_fetch_of_previous_one() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          count: add(num: 1)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "c") {
          mutation {
            count: add(num: 1)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Same as above, with the entity fetch behind `@include`.
#[test]
fn next_field_waits_for_conditional_entity_fetch_of_previous_one() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation ($includeRemote: Boolean!) {
          create: addProduct(input: { name: "new", price: 599.99 }) {
            ... on Product @include(if: $includeRemote) {
              isExpensive
            }
          }
          count: add(num: 1)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation ($includeRemote:Boolean!) {
            create: addProduct(input: {name: "new", price: 599.99}) {
              ... on Product @include(if: $includeRemote) {
                __typename
                id
                price
              }
            }
          }
        },
        Flatten(path: "create|[Product]") {
          Fetch(service: "b", if: $includeRemote) {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "c") {
          mutation {
            count: add(num: 1)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `create` and `multiply` are both in `a`, so they go in one request, like in Apollo, even though
/// `create` needs `b` for `isExpensive`. `b` runs after that request, once the product exists.
#[test]
fn neighbours_in_one_graph_share_a_fetch_when_first_needs_another_fetch(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          double: multiply(by: 2)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
            double: multiply(by: 2)
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

/// `create` and `double` go to `a` in one request. `count` is in `c`, so it waits for the whole
/// group, `isExpensive` from `b` included.
#[test]
fn next_graph_waits_for_entity_fetch_of_the_group() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          double: multiply(by: 2)
          count: add(num: 1)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
            double: multiply(by: 2)
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "c") {
          mutation {
            count: add(num: 1)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Same as above, with the entity fetch on the second field of the group.
#[test]
fn next_graph_waits_for_entity_fetch_of_a_later_field_in_the_group() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          double: multiply(by: 2)
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          count: add(num: 1)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            double: multiply(by: 2)
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "c") {
          mutation {
            count: add(num: 1)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `final` is in `b`, like the entity fetch for `isExpensive`. It's a mutation of its own, so it
/// still waits for that fetch, and the two stay apart.
#[test]
fn next_graph_waits_for_entity_fetch_to_the_same_graph() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          double: multiply(by: 2)
          final: delete
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
            double: multiply(by: 2)
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "b") {
          mutation {
            final: delete
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `one` and `two` are both in `c`, but `create` is between them, so they aren't a group.
#[test]
fn fields_of_one_graph_with_another_graph_between_stay_apart() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation {
          one: add(num: 1)
          create: addProduct(input: { name: "new", price: 599.99 }) {
            isExpensive
          }
          two: add(num: 2)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "c") {
          mutation {
            one: add(num: 1)
          }
        },
        Fetch(service: "a") {
          mutation {
            create: addProduct(input: {name: "new", price: 599.99}) {
              __typename
              id
              price
            }
          }
        },
        Flatten(path: "create") {
          Fetch(service: "b") {
            {
              ... on Product {
                __typename
                price
                id
              }
            } =>
            {
              ... on Product {
                isExpensive
              }
            }
          },
        },
        Fetch(service: "c") {
          mutation {
            two: add(num: 2)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// A field under `@include` still goes in the request of its group.
#[test]
fn conditional_field_stays_in_its_group() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation ($x: Boolean!) {
          one: add(num: 1) @include(if: $x)
          two: add(num: 2)
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "c") {
        mutation ($x:Boolean!) {
          one: add(num: 1) @include(if: $x)
          two: add(num: 2)
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn shareable_mutation_field_runs_once() -> Result<(), Box<dyn Error>> {
    init_logger();
    // `createOrder` lives in both subgraphs, but `total` only in a and `eta` only in b.
    // The mutation must hit one subgraph, the rest comes through an entity call.
    let supergraph = r#"
        enum join__Graph {
            A @join__graph(name: "a", url: "")
            B @join__graph(name: "b", url: "")
        }
        type Query @join__type(graph: A) @join__type(graph: B) {
            ping: Boolean @join__field(graph: A)
        }
        type Mutation @join__type(graph: A) @join__type(graph: B) {
            createOrder: Order
        }
        type Order @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            total: Int @join__field(graph: A)
            eta: String @join__field(graph: B)
        }
    "#;
    let query_plan = build_query_plan_from_sdl(
        supergraph,
        parse_operation("mutation { createOrder { total eta } }"),
        Default::default(),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            createOrder {
              __typename
              total
              id
            }
          }
        },
        Flatten(path: "createOrder") {
          Fetch(service: "b") {
            {
              ... on Order {
                __typename
                id
              }
            } =>
            {
              ... on Order {
                eta
              }
            }
          },
        },
      },
    },
    "#);

    // everything is in b, so no reason to start in a
    let query_plan = build_query_plan_from_sdl(
        supergraph,
        parse_operation("mutation { createOrder { id eta } }"),
        Default::default(),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        mutation {
          createOrder {
            id
            eta
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn mutation_key_does_not_jump_between_subgraphs() -> Result<(), Box<dyn Error>> {
    init_logger();
    // `Mutation` has a key in both subgraphs. Jumping from one `Mutation` to the other
    // through `_entities` would run `createOrder` twice.
    // b can't be jumped to for `Order`, so the mutation has to start in b.
    let supergraph = r#"
        enum join__Graph {
            A @join__graph(name: "a", url: "")
            B @join__graph(name: "b", url: "")
        }
        type Query @join__type(graph: A) @join__type(graph: B) {
            ping: Boolean @join__field(graph: A)
        }
        type Mutation @join__type(graph: A, key: "__typename") @join__type(graph: B, key: "__typename") {
            createOrder: Order
        }
        type Order @join__type(graph: A, key: "id") @join__type(graph: B, key: "id", resolvable: false) {
            id: ID!
            total: Int @join__field(graph: A)
            eta: String @join__field(graph: B)
        }
    "#;
    let query_plan = build_query_plan_from_sdl(
        supergraph,
        parse_operation("mutation { createOrder { total eta } }"),
        Default::default(),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          mutation {
            createOrder {
              __typename
              id
              eta
            }
          }
        },
        Flatten(path: "createOrder") {
          Fetch(service: "a") {
            {
              ... on Order {
                __typename
                id
              }
            } =>
            {
              ... on Order {
                total
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn shareable_mutation_field_in_conditional_fragment_runs_once() -> Result<(), Box<dyn Error>> {
    init_logger();
    // The fragment keeps its @include, so it reaches the planner as one root selection.
    // No subgraph has all of it (`cancelOrder` is only in c), but each mutation field
    // inside it still has to run once, and in order.
    let supergraph = r#"
        enum join__Graph {
            A @join__graph(name: "a", url: "")
            B @join__graph(name: "b", url: "")
            C @join__graph(name: "c", url: "")
        }
        type Query @join__type(graph: A) @join__type(graph: B) @join__type(graph: C) {
            ping: Boolean @join__field(graph: A)
        }
        type Mutation @join__type(graph: A) @join__type(graph: B) @join__type(graph: C) {
            createOrder: Order @join__field(graph: A) @join__field(graph: B)
            cancelOrder: Boolean @join__field(graph: C)
        }
        type Order @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            total: Int @join__field(graph: A)
            eta: String @join__field(graph: B)
        }
    "#;
    let query_plan = build_query_plan_from_sdl(
        supergraph,
        parse_operation(
            r#"
            mutation ($run: Boolean!) {
              ... on Mutation @include(if: $run) {
                createOrder { total eta }
                cancelOrder
              }
            }
            "#,
        ),
        Default::default(),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a", if: $run) {
          mutation ($run:Boolean!) {
            ... on Mutation @include(if: $run) {
              createOrder {
                __typename
                total
                id
              }
            }
          }
        },
        Flatten(path: "|[Mutation].createOrder") {
          Fetch(service: "b", if: $run) {
            {
              ... on Order {
                __typename
                id
              }
            } =>
            {
              ... on Order {
                eta
              }
            }
          },
        },
        Fetch(service: "c", if: $run) {
          mutation ($run:Boolean!) {
            ... on Mutation @include(if: $run) {
              cancelOrder
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn shareable_mutation_field_picks_subgraph_by_whole_plan() -> Result<(), Box<dyn Error>> {
    init_logger();
    // From a, `l1`, `l2` and `l3` come in one entity call to c.
    // From b, they're local, but `x` takes two calls in a row (b -> e -> d).
    // Counted per leaf, a looks more expensive (three trips vs one longer one),
    // but its plan has fewer fetches, so a it is.
    let supergraph = r#"
        enum join__Graph {
            A @join__graph(name: "a", url: "")
            B @join__graph(name: "b", url: "")
            C @join__graph(name: "c", url: "")
            D @join__graph(name: "d", url: "")
            E @join__graph(name: "e", url: "")
        }
        type Query @join__type(graph: A) @join__type(graph: B) @join__type(graph: C) @join__type(graph: D) @join__type(graph: E) {
            ping: Boolean @join__field(graph: A)
        }
        type Mutation @join__type(graph: A) @join__type(graph: B) {
            createOrder: Order
        }
        type Order
            @join__type(graph: A, key: "id", resolvable: false)
            @join__type(graph: B, key: "id", resolvable: false)
            @join__type(graph: C, key: "id")
            @join__type(graph: D, key: "code")
            @join__type(graph: E, key: "id")
            @join__type(graph: E, key: "code")
        {
            id: ID! @join__field(graph: A) @join__field(graph: B) @join__field(graph: C) @join__field(graph: E)
            code: ID! @join__field(graph: D) @join__field(graph: E)
            x: Int @join__field(graph: A) @join__field(graph: D)
            l1: Int @join__field(graph: B) @join__field(graph: C)
            l2: Int @join__field(graph: B) @join__field(graph: C)
            l3: Int @join__field(graph: B) @join__field(graph: C)
        }
    "#;
    let query_plan = build_query_plan_from_sdl(
        supergraph,
        parse_operation("mutation { createOrder { x l1 l2 l3 } }"),
        Default::default(),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a") {
          mutation {
            createOrder {
              __typename
              x
              id
            }
          }
        },
        Flatten(path: "createOrder") {
          Fetch(service: "c") {
            {
              ... on Order {
                __typename
                id
              }
            } =>
            {
              ... on Order {
                l1
                l2
                l3
              }
            }
          },
        },
      },
    },
    "#);

    Ok(())
}
