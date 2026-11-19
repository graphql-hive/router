use crate::query_planner::{
    planner::{
        plan_nodes::{
            planning::{PlanNode, QueryPlan},
            EntityReuse, FetchId,
        },
        QueryPlannerOptions,
    },
    tests::testkit::{build_query_plan, build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

/// Plans with entity reuse on, it's off by default
fn plan_with_reuse(
    fixture_path: &str,
    document: graphql_tools::parser::query::Document<'static, String>,
) -> Result<QueryPlan, Box<dyn Error>> {
    build_query_plan(
        fixture_path,
        document,
        Default::default(),
        QueryPlannerOptions {
            entity_reuse: true,
            ..Default::default()
        },
    )
}

/// Entity reuse with `split_nested_entities` on
fn plan_with_split(
    fixture_path: &str,
    document: graphql_tools::parser::query::Document<'static, String>,
) -> Result<QueryPlan, Box<dyn Error>> {
    build_query_plan(
        fixture_path,
        document,
        Default::default(),
        QueryPlannerOptions {
            entity_reuse: true,
            split_nested_entities: true,
            ..Default::default()
        },
    )
}

/// One line per fetch (a batch, one per alias): its id, where it merges, where it may copy from,
/// and the fetches it waits for. A root fetch merges at `root`. A batch lists every fetch it
/// completes, so each id in a wait list is one of the lines.
fn describe(plan: &QueryPlan) -> String {
    // each fetch of the plan, then its follow-ups by level, indented
    fn walk(plan: &QueryPlan, node: &PlanNode, position: &mut u32, out: &mut String) {
        if let PlanNode::Sequence(s) = node {
            return s.nodes.iter().for_each(|n| walk(plan, n, position, out));
        }
        if let PlanNode::Parallel(p) = node {
            return p.nodes.iter().for_each(|n| walk(plan, n, position, out));
        }
        print(node, out);
        let mut level = 0;
        while let Some(nodes) = plan.follow_ups(*position, level) {
            for node in nodes {
                let mut lines = String::new();
                print(node, &mut lines);
                for line in lines.lines() {
                    out.push_str(&format!("  then ({level}) {line}\n"));
                }
            }
            level += 1;
        }
        *position += 1;
    }
    fn print(node: &PlanNode, out: &mut String) {
        match node {
            PlanNode::Fetch(fetch) => line(
                out,
                &[fetch.id],
                &fetch.service_name,
                "root",
                fetch.entity_reuse.as_deref(),
                &fetch.depends_on,
            ),
            PlanNode::Flatten(flatten) => {
                if let PlanNode::Fetch(fetch) = flatten.node.as_ref() {
                    line(
                        out,
                        &[fetch.id],
                        &fetch.service_name,
                        &flatten.path.to_string(),
                        fetch.entity_reuse.as_deref(),
                        &fetch.depends_on,
                    );
                }
            }
            PlanNode::BatchFetch(batch) => {
                let mut completes = batch.completes.to_vec();
                completes.sort_unstable();
                for alias in &batch.entity_batch.aliases {
                    let paths: Vec<String> =
                        alias.merge_paths.iter().map(|p| p.to_string()).collect();
                    line(
                        out,
                        &completes,
                        &batch.service_name,
                        &paths.join(" + "),
                        alias.entity_reuse.as_deref(),
                        &batch.depends_on,
                    );
                }
            }
            _ => {}
        }
    }
    fn line(
        out: &mut String,
        ids: &[FetchId],
        service: &str,
        path: &str,
        reuse: Option<&EntityReuse>,
        waits: &[FetchId],
    ) {
        let sources = match reuse {
            Some(reuse) => reuse
                .sources
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            None => "-".to_string(),
        };
        let list = |ids: &[FetchId]| {
            ids.iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "[{}] {service} at {path} <- {sources} after [{}]\n",
            list(ids),
            list(waits)
        ));
    }
    let mut out = String::new();
    if let Some(node) = plan.node() {
        walk(plan, node, &mut 0, &mut out);
    }
    out
}

#[test]
fn bench_operation_entities_can_come_from_earlier_fetches() -> Result<(), Box<dyn Error>> {
    init_logger();
    let operation = std::fs::read_to_string("../../bench/operation.graphql")?;
    let query_plan = plan_with_reuse(
        "../../bench/supergraph.graphql",
        parse_operation(&operation),
    )?;

    // the deep `inventory` batch [22] asks what the call at `topProducts` [17] asked, so it waits
    // for that one too and copies. without the wait the copy would be a race the deep call loses.
    insta::assert_snapshot!(describe(&query_plan), @r"
    [15] products at root <- - after []
    [2] accounts at root <- - after []
    [17] inventory at topProducts.@ <- - after [15]
    [3, 20] reviews at topProducts.@ <- - after [2, 15]
    [3, 20] reviews at users.@ <- - after [2, 15]
    [7, 13, 25] products at topProducts.@.reviews.@.author.reviews.@.product + users.@.reviews.@.product + users.@.reviews.@.product.reviews.@.author.reviews.@.product <- topProducts.@ after [3, 20]
    [9, 21] accounts at topProducts.@.reviews.@.author + users.@.reviews.@.product.reviews.@.author <- users.@ after [3, 20]
    [4, 10, 22] inventory at topProducts.@.reviews.@.author.reviews.@.product + users.@.reviews.@.product + users.@.reviews.@.product.reviews.@.author.reviews.@.product <- topProducts.@ after [7, 13, 17, 25]
    ");

    // with `split_nested_entities` on, `reviews` [3, 20] only asks for one level of reviews.
    // Its follow-ups copy the deeper ones from `users` and `topProducts`, what [3, 20] put
    // there: [28, 29] first, then [27] under what [28] filled in. The fetches after [3, 20] wait
    // for those too, the rest of the plan is the same.
    let query_plan = plan_with_split(
        "../../bench/supergraph.graphql",
        parse_operation(&operation),
    )?;
    insta::assert_snapshot!(describe(&query_plan), @r"
    [15] products at root <- - after []
    [2] accounts at root <- - after []
    [17] inventory at topProducts.@ <- - after [15]
    [3, 20] reviews at topProducts.@ <- - after [2, 15]
    [3, 20] reviews at users.@ <- - after [2, 15]
      then (0) [28, 29] reviews at topProducts.@.reviews.@.author <- topProducts.@.reviews.@.author, users.@, users.@.reviews.@.product.reviews.@.author after []
      then (0) [28, 29] reviews at users.@.reviews.@.product <- topProducts.@, users.@.reviews.@.product after []
      then (1) [27] reviews at users.@.reviews.@.product.reviews.@.author <- topProducts.@.reviews.@.author, users.@, users.@.reviews.@.product.reviews.@.author after []
    [7, 13, 25] products at topProducts.@.reviews.@.author.reviews.@.product + users.@.reviews.@.product + users.@.reviews.@.product.reviews.@.author.reviews.@.product <- topProducts.@ after [3, 20]
    [9, 21] accounts at topProducts.@.reviews.@.author + users.@.reviews.@.product.reviews.@.author <- users.@ after [3, 20]
    [4, 10, 22] inventory at topProducts.@.reviews.@.author.reviews.@.product + users.@.reviews.@.product + users.@.reviews.@.product.reviews.@.author.reviews.@.product <- topProducts.@ after [7, 13, 17, 25]
    ");

    // with reuse off, nothing copies
    let query_plan = build_query_plan_with_defaults(
        "../../bench/supergraph.graphql",
        parse_operation(&operation),
    )?;
    assert!(!describe(&query_plan).contains("<- topProducts"));
    assert!(!describe(&query_plan).contains("<- users"));

    Ok(())
}

#[test]
fn source_with_an_aliased_key_is_left_out() -> Result<(), Box<dyn Error>> {
    init_logger();
    // `id` on users is the username, the executor would match authors by it
    let document =
        parse_operation(r#"{ users { id: username name reviews { author { name } } } }"#);
    let query_plan = plan_with_reuse("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [2] accounts at root <- - after []
    [3] reviews at users.@ <- - after [2]
    [4] accounts at users.@.reviews.@.author <- - after [3]
    ");

    Ok(())
}

#[test]
fn mutation_fields_never_copy_from_each_other() -> Result<(), Box<dyn Error>> {
    init_logger();
    // `add` keeps the two products in separate waves. A mutation can change what an earlier one
    // returned, so `second` can't copy from `first`.
    let document = parse_operation(
        r#"mutation {
          first: addProduct(input: { name: "a", price: 1 }) { id isAvailable }
          add(num: 1)
          second: addProduct(input: { name: "b", price: 2 }) { id isAvailable }
        }"#,
    );
    let query_plan = plan_with_reuse("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [2] a at root <- - after []
    [3] b at first <- - after [2]
    [4] c at root <- - after [3]
    [5] a at root <- - after [4]
    [6] b at second <- - after [5]
    ");

    Ok(())
}

#[test]
fn never_copies_from_a_fetch_it_does_not_depend_on() -> Result<(), Box<dyn Error>> {
    init_logger();
    // `bFeed` has `createdAt` from `b`, the subgraph the plan asks for it at `aFeed`. But the
    // `aFeed` call only waits for `aFeed`, so `bFeed` might not be in yet when it runs
    let document = parse_operation("{ aFeed { createdAt } bFeed { createdAt } }");
    let query_plan = plan_with_reuse(
        "fixture/tests/simple-progressive-overrides.supergraph.graphql",
        document,
    )?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [4] b at root <- - after []
    [2] a at root <- - after []
    [3] b at aFeed.@ <- - after [2]
    ");

    Ok(())
}

#[test]
fn an_alias_outside_the_fragment_rules_out_its_objects() -> Result<(), Box<dyn Error>> {
    init_logger();
    // the books at `products` get `id` from `id: sku`, outside `... on Book`. A representation
    // read there would hold a sku, not the book's id, so the deep books can't copy from them.
    // (the fetch puts `id: sku` into `... on Book`, which the conflict check sees, and `walk`
    // leaves out objects whose fields aren't all in their fragment)
    let document = parse_operation(
        r#"{
          products {
            id: sku
            ... on Book { title reviews { product { ... on Book { title } } } }
          }
        }"#,
    );
    let query_plan = plan_with_reuse("fixture/tests/abstract-types.supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [2] products at root <- - after []
    [4] reviews at products.@|[Book] <- - after [2]
    [3] books at products.@|[Book] <- - after [2]
    [5] books at products.@|[Book].reviews.@.product|[Book] <- - after [4]
    ");

    Ok(())
}

// `users` has every author's reviews, but its key is renamed: `id` holds the username there,
// so no author would ever match one of them. Split out, the authors' reviews would always be
// one more call, so they stay in.
#[test]
fn nested_entity_stays_when_the_source_key_is_aliased() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"{
          users { id: username username: id reviews { body } }
          topProducts { reviews { author { id username reviews { body } } } }
        }"#,
    );
    let query_plan = plan_with_split("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [4] products at root <- - after []
    [2] accounts at root <- - after []
    [3, 5] reviews at topProducts.@ <- - after [2, 4]
    [3, 5] reviews at users.@ <- - after [2, 4]
    ");

    Ok(())
}

// The author's key isn't asked for, `id` holds the username here. Adding `id` to the call would
// put the author's id where the username goes, so it stays in one call.
#[test]
fn nested_entity_stays_when_its_key_is_not_asked_for() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"{
          users { reviews { body } }
          topProducts { reviews { author { id: username reviews { body } } } }
        }"#,
    );
    let query_plan = plan_with_split("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [4] products at root <- - after []
    [2] accounts at root <- - after []
    [3, 5] reviews at topProducts.@ <- - after [2, 4]
    [3, 5] reviews at users.@ <- - after [2, 4]
    ");

    Ok(())
}

// `reviews` would send each author's reviews again for every review of a top product. `users`
// already has them, so the call at `topProducts` stops at the author's key and a fetch of its
// own copies the rest. `username` stays: `reviews` only has it through `@provides`.
#[test]
fn nested_entity_is_split_out_when_another_fetch_has_its_fields() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"{
          users { reviews { body } }
          topProducts { reviews { author { id username reviews { body } } } }
        }"#,
    );
    let query_plan = plan_with_split("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [4] products at root <- - after []
    [2] accounts at root <- - after []
    [3, 5] reviews at topProducts.@ <- - after [2, 4]
    [3, 5] reviews at users.@ <- - after [2, 4]
      then (0) [6] reviews at topProducts.@.reviews.@.author <- topProducts.@.reviews.@.author, users.@ after []
    ");
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "products") {
            {
              topProducts {
                __typename
                upc
              }
            }
          },
          Fetch(service: "accounts") {
            {
              users {
                __typename
                id
              }
            }
          },
        },
        BatchFetch(service: "reviews") {
          {
            _e0 {
              paths: [
                "topProducts.@"
              ]
              {
                ... on Product {
                  __typename
                  upc
                }
              }
            }
            _e1 {
              paths: [
                "users.@"
              ]
              {
                ... on User {
                  __typename
                  id
                }
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Product {
                reviews {
                  author {
                    __typename
                    id
                    username
                  }
                }
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on User {
                reviews {
                  body
                }
              }
            }
          }
        },
      },
      FollowUps(after: 2) {
        Level {
          Flatten(path: "topProducts.@.reviews.@.author") {
            Fetch(service: "reviews") {
              {
                ... on User {
                  __typename
                  id
                }
              } =>
              {
                ... on User {
                  reviews {
                    body
                  }
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

// nothing else has the authors' reviews, a fetch of their own would be one more call
#[test]
fn nested_entity_stays_without_a_source() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document =
        parse_operation(r#"{ topProducts { reviews { author { reviews { body } } } } }"#);
    let query_plan = plan_with_split("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [2] products at root <- - after []
    [3] reviews at topProducts.@ <- - after [2]
    ");

    Ok(())
}

// the split only pays off when the fetch copies, with reuse off nothing is split
#[test]
fn nested_entity_stays_with_reuse_off() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"{
          users { reviews { body } }
          topProducts { reviews { author { username reviews { body } } } }
        }"#,
    );
    let query_plan = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [4] products at root <- - after []
    [2] accounts at root <- - after []
    [3, 5] reviews at topProducts.@ <- - after [2, 4]
    [3, 5] reviews at users.@ <- - after [2, 4]
    ");

    Ok(())
}

// with entity reuse on but `split_nested_entities` off, nothing is split
#[test]
fn nested_entity_stays_with_split_off() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"{
          users { reviews { body } }
          topProducts { reviews { author { id username reviews { body } } } }
        }"#,
    );
    let query_plan = plan_with_reuse("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [4] products at root <- - after []
    [2] accounts at root <- - after []
    [3, 5] reviews at topProducts.@ <- - after [2, 4]
    [3, 5] reviews at users.@ <- - after [2, 4]
    ");

    Ok(())
}

// the e2e test for a split fetch that doesn't find its entity uses this plan: the `reviews`
// call for `me` and `topProducts` is one batch, and the authors' reviews copy from `me`
#[test]
fn nested_entity_copies_from_a_single_entity() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        r#"{
          me { id reviews { body } }
          topProducts(first: 1) { reviews { author { id reviews { body } } } }
        }"#,
    );
    let query_plan = plan_with_split("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    [4] products at root <- - after []
    [2] accounts at root <- - after []
    [3, 5] reviews at topProducts.@ <- - after [2, 4]
    [3, 5] reviews at me <- - after [2, 4]
      then (0) [6] reviews at topProducts.@.reviews.@.author <- topProducts.@.reviews.@.author, me after []
    ");
    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "products") {
            {
              topProducts(first: 1) {
                __typename
                upc
              }
            }
          },
          Fetch(service: "accounts") {
            {
              me {
                __typename
                id
              }
            }
          },
        },
        BatchFetch(service: "reviews") {
          {
            _e0 {
              paths: [
                "topProducts.@"
              ]
              {
                ... on Product {
                  __typename
                  upc
                }
              }
            }
            _e1 {
              paths: [
                "me"
              ]
              {
                ... on User {
                  __typename
                  id
                }
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Product {
                reviews {
                  author {
                    __typename
                    id
                  }
                }
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on User {
                reviews {
                  body
                }
              }
            }
          }
        },
      },
      FollowUps(after: 2) {
        Level {
          Flatten(path: "topProducts.@.reviews.@.author") {
            Fetch(service: "reviews") {
              {
                ... on User {
                  __typename
                  id
                }
              } =>
              {
                ... on User {
                  reviews {
                    body
                  }
                }
              }
            },
          },
        },
      },
    },
    "#);

    // the exposed plan shows them too, after the batch, the fetch at position 2
    let json = serde_json::to_value(&query_plan)?;
    assert_eq!(json["followUps"][0]["after"], 2);
    assert_eq!(json["followUps"][0]["levels"][0][0]["kind"], "Flatten");

    Ok(())
}
