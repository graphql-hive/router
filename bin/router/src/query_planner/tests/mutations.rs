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

/// `create` and `multiply` are both in `a`, but `create` needs `b` for `isExpensive`.
/// A mutation field is done only when all of its data is, so `double` waits for `b`
/// in a request of its own, and can't change what `b` reads.
#[test]
fn neighbours_in_one_graph_wait_for_the_first_ones_entity_fetch() -> Result<(), Box<dyn Error>> {
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
        Fetch(service: "a") {
          mutation {
            double: multiply(by: 2)
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `create` needs `b` for `isExpensive`, so `double` waits for it in a request of its own,
/// and `count` in `c` waits for `double`.
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
        Fetch(service: "a") {
          mutation {
            double: multiply(by: 2)
          }
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
        Fetch(service: "a") {
          mutation {
            double: multiply(by: 2)
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

#[test]
fn shareable_mutation_fields_stay_in_one_fetch() -> Result<(), Box<dyn Error>> {
    init_logger();
    // Both subgraphs can run both fields, but only b has `customer`.
    // Back to back mutation fields in the same subgraph go in one fetch,
    // so `cancelOrder` goes to b too, even though a could run it just as well on its own.
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
            cancelOrder(id: ID): Boolean
        }
        type Order @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            total: Int
            customer: Customer @join__field(graph: B)
        }
        type Customer @join__type(graph: B) {
            name: String
        }
    "#;
    let query_plan = build_query_plan_from_sdl(
        supergraph,
        parse_operation(
            "mutation { createOrder { id total customer { name } } cancelOrder(id: 1) }",
        ),
        Default::default(),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        mutation {
          createOrder {
            id
            total
            customer {
              name
            }
          }
          cancelOrder(id: 1)
        }
      },
    },
    "#);

    // same the other way around
    let query_plan = build_query_plan_from_sdl(
        supergraph,
        parse_operation(
            "mutation { cancelOrder(id: 1) createOrder { id total customer { name } } }",
        ),
        Default::default(),
        Default::default(),
    )?;
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Fetch(service: "b") {
        mutation {
          cancelOrder(id: 1)
          createOrder {
            id
            total
            customer {
              name
            }
          }
        }
      },
    },
    "#);

    Ok(())
}

#[test]
fn shareable_mutation_field_between_fields_of_one_subgraph() -> Result<(), Box<dyn Error>> {
    init_logger();
    // `first` and `third` only run in b. `second` reads `total` right in a.
    // In b it'd need an entity call to a, and `third` has to wait for that,
    // as `second` is done only when `total` is. So b can't run all three in one request,
    // and `second` goes to a.
    let supergraph = r#"
        enum join__Graph {
            A @join__graph(name: "a", url: "")
            B @join__graph(name: "b", url: "")
        }
        type Query @join__type(graph: A) @join__type(graph: B) {
            ping: Boolean @join__field(graph: A)
        }
        type Mutation @join__type(graph: A) @join__type(graph: B) {
            first: Boolean @join__field(graph: B)
            second: Order
            third: Boolean @join__field(graph: B)
        }
        type Order @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            total: Int @join__field(graph: A)
        }
    "#;
    let query_plan = build_query_plan_from_sdl(
        supergraph,
        parse_operation("mutation { first second { total } third }"),
        Default::default(),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          mutation {
            first
          }
        },
        Fetch(service: "a") {
          mutation {
            second {
              total
            }
          }
        },
        Fetch(service: "b") {
          mutation {
            third
          }
        },
      },
    },
    "#);

    Ok(())
}

#[test]
fn mutation_field_waits_for_entity_fetch_of_the_field_before() -> Result<(), Box<dyn Error>> {
    init_logger();
    // All three run in b, but `second` needs a for `total`. `second` is done only when `total`
    // is, so `third` waits for that call, and can't change what a reads.
    let supergraph = r#"
        enum join__Graph {
            A @join__graph(name: "a", url: "")
            B @join__graph(name: "b", url: "")
        }
        type Query @join__type(graph: A) @join__type(graph: B) {
            ping: Boolean @join__field(graph: A)
        }
        type Mutation @join__type(graph: B) {
            first: Boolean
            second: Order
            third: Boolean
        }
        type Order @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            total: Int @join__field(graph: A)
        }
    "#;
    let query_plan = build_query_plan_from_sdl(
        supergraph,
        parse_operation("mutation { first second { total } third }"),
        Default::default(),
        Default::default(),
    )?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "b") {
          mutation {
            first
            second {
              __typename
              id
            }
          }
        },
        Flatten(path: "second") {
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
        Fetch(service: "b") {
          mutation {
            third
          }
        },
      },
    },
    "#);

    Ok(())
}

/// `create` is selected twice under one response key, so GraphQL runs it once, with both
/// selections. With both conditions on, `a` gets one request that has both.
#[test]
fn field_selected_twice_in_a_conditional_fragment_runs_once() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation ($a: Boolean!, $b: Boolean!) {
          ... on Mutation @include(if: $a) {
            create: addProduct(input: { name: "new", price: 1 }) @include(if: $b) { isExpensive }
            create: addProduct(input: { name: "new", price: 1 }) { id }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a", if: $a) {
          mutation ($a:Boolean!,$b:Boolean!) {
            ... on Mutation @include(if: $a) {
              create: addProduct(input: {name: "new", price: 1}) @include(if: $b) {
                __typename
                id
                price
              }
              create: addProduct(input: {name: "new", price: 1}) {
                id
              }
            }
          }
        },
        Flatten(path: "|[Mutation].create") {
          Fetch(service: "b", if: $a && $b) {
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

/// Same, with each selection in a root fragment of its own condition.
#[test]
fn field_selected_in_two_conditional_fragments_runs_once() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation ($a: Boolean!, $b: Boolean!) {
          ... on Mutation @include(if: $a) {
            create: addProduct(input: { name: "new", price: 1 }) { isExpensive }
          }
          ... on Mutation @include(if: $b) {
            create: addProduct(input: { name: "new", price: 1 }) { id }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a", if: $a || $b) {
          mutation ($a:Boolean!,$b:Boolean!) {
            ... on Mutation @include(if: $a) {
              create: addProduct(input: {name: "new", price: 1}) {
                __typename
                id
                ... on Product @include(if: $a) {
                  __typename
                  price
                  id
                }
              }
            }
            ... on Mutation @include(if: $b) {
              create: addProduct(input: {name: "new", price: 1}) {
                id
              }
            }
          }
        },
        Flatten(path: "|[Mutation].create") {
          Fetch(service: "b", if: $a) {
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

/// The field runs at its first selection that's on, with what every selection that's on asks
/// for. With `$b` that's before `count`, without it after: two calls to `a` that never both run.
#[test]
fn field_selected_twice_with_another_between_runs_at_its_first_selection_that_is_on(
) -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation ($b: Boolean!) {
          create: addProduct(input: { name: "new", price: 1 }) @include(if: $b) { isExpensive }
          count: add(num: 1)
          create: addProduct(input: { name: "new", price: 1 }) { id }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a", if: $b) {
          mutation ($b:Boolean!) {
            ... on Mutation @include(if: $b) {
              create: addProduct(input: {name: "new", price: 1}) {
                __typename
                id
                ... on Product @include(if: $b) {
                  __typename
                  price
                  id
                }
              }
            }
          }
        },
        Flatten(path: "|[Mutation].create") {
          Fetch(service: "b", if: $b) {
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
        Fetch(service: "a", if: !$b) {
          mutation ($b:Boolean!) {
            ... on Mutation @skip(if: $b) {
              create: addProduct(input: {name: "new", price: 1}) {
                id
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Each selection of `create` here has two conditions and another field after it. Saying
/// "every one before is off" for the last one takes 2^20 copies, so the operation is turned
/// down before any get made.
#[test]
fn field_selected_under_too_many_conditions_is_turned_down() {
    init_logger();
    let mut variables = Vec::new();
    let mut selections = String::new();
    for index in 0..20 {
        variables.push(format!("$a{index}: Boolean!, $b{index}: Boolean!"));
        selections.push_str(&format!(
            r#"create: addProduct(input: {{ name: "new", price: 1 }}) @include(if: $a{index}) @skip(if: $b{index}) {{ id }}
            count{index}: add(num: {index})
            "#
        ));
    }
    selections.push_str(r#"create: addProduct(input: { name: "new", price: 1 }) { id }"#);
    let document = parse_operation(&format!(
        "mutation ({}) {{ {selections} }}",
        variables.join(", ")
    ));

    let started = std::time::Instant::now();
    let error =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)
            .expect_err("too many conditional copies");
    assert!(
        error
            .to_string()
            .contains("too many different @skip/@include conditions"),
        "{error}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

/// The two `create`s sit in root fragments with the same condition, `count` between them.
/// Merging the fragments into the first would run the second `create` before `count`, so they
/// stay apart, and `create` runs before `count` only when its first selection is on.
#[test]
fn equal_root_fragments_around_another_field_keep_their_order() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation ($outer: Boolean!, $b: Boolean!) {
          ... on Mutation @include(if: $outer) {
            create: addProduct(input: { name: "new", price: 1 }) @include(if: $b) { isExpensive }
          }
          count: add(num: 1)
          ... on Mutation @include(if: $outer) {
            create: addProduct(input: { name: "new", price: 1 }) { id }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a", if: $b && $outer) {
          mutation ($b:Boolean!,$outer:Boolean!) {
            ... on Mutation @include(if: $b) {
              ... on Mutation @include(if: $outer) {
                create: addProduct(input: {name: "new", price: 1}) {
                  __typename
                  id
                  ... on Product @include(if: $b) {
                    __typename
                    price
                    id
                  }
                }
              }
            }
          }
        },
        Flatten(path: "|[Mutation]|[Mutation].create") {
          Fetch(service: "b", if: $b && $outer) {
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
        Fetch(service: "a", if: $outer && !$b) {
          mutation ($b:Boolean!,$outer:Boolean!) {
            ... on Mutation @skip(if: $b) {
              ... on Mutation @include(if: $outer) {
                create: addProduct(input: {name: "new", price: 1}) {
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

/// The same, with both fragments inside another conditional one: a fragment is still the
/// root, so they don't merge across `count` either.
#[test]
fn equal_nested_root_fragments_around_another_field_keep_their_order() -> Result<(), Box<dyn Error>>
{
    init_logger();
    let document = parse_operation(
        r#"
        mutation ($wrap: Boolean!, $outer: Boolean!, $b: Boolean!) {
          ... on Mutation @include(if: $wrap) {
            ... on Mutation @include(if: $outer) {
              create: addProduct(input: { name: "new", price: 1 }) @include(if: $b) { isExpensive }
            }
            count: add(num: 1)
            ... on Mutation @include(if: $outer) {
              create: addProduct(input: { name: "new", price: 1 }) { id }
            }
          }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a", if: $b && $outer && $wrap) {
          mutation ($b:Boolean!,$outer:Boolean!,$wrap:Boolean!) {
            ... on Mutation @include(if: $b) {
              ... on Mutation @include(if: $wrap) {
                ... on Mutation @include(if: $outer) {
                  create: addProduct(input: {name: "new", price: 1}) {
                    __typename
                    id
                    ... on Product @include(if: $b) {
                      __typename
                      price
                      id
                    }
                  }
                }
              }
            }
          }
        },
        Flatten(path: "|[Mutation]|[Mutation]|[Mutation].create") {
          Fetch(service: "b", if: $b && $outer && $wrap) {
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
        Fetch(service: "c", if: $wrap) {
          mutation ($wrap:Boolean!) {
            ... on Mutation @include(if: $wrap) {
              count: add(num: 1)
            }
          }
        },
        Fetch(service: "a", if: $outer && $wrap && !$b) {
          mutation ($b:Boolean!,$outer:Boolean!,$wrap:Boolean!) {
            ... on Mutation @skip(if: $b) {
              ... on Mutation @include(if: $wrap) {
                ... on Mutation @include(if: $outer) {
                  create: addProduct(input: {name: "new", price: 1}) {
                    id
                  }
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

/// A selection that `@include(if: $b) @skip(if: $b)` never lets run has no spot to take, and
/// the rest of the mutation plans as without it.
#[test]
fn selection_that_never_runs_is_left_out() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"
        mutation ($a: Boolean!, $b: Boolean!) {
          create: addProduct(input: { name: "new", price: 1 }) @include(if: $a) { id }
          count: add(num: 1)
          create: addProduct(input: { name: "new", price: 1 }) @include(if: $b) @skip(if: $b) { id }
        }
        "#,
    );
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Fetch(service: "a", if: $a) {
          mutation ($a:Boolean!) {
            create: addProduct(input: {name: "new", price: 1}) @include(if: $a) {
              id
            }
          }
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
