mod alias;
mod arguments;
mod context;
mod dependencies;
mod entity_reuse;
mod fragments;
mod generated;
mod include_skip;
mod interface;
mod interface_object;
mod interface_object_with_requires;
mod issues;
mod mutations;
mod object_entities;
mod override_requires;
mod overrides;
mod provides;
mod renamed_root_types;
mod requires;
mod requires_alias_scopes;
mod requires_circular;
mod requires_fragments;
mod requires_provides;
mod requires_requires;
mod root_reentry;
mod root_types;
pub(crate) mod testkit;
mod union;

use crate::query_planner::{
    tests::testkit::{build_query_plan_with_defaults, init_logger, read_supergraph_state},
    utils::parsing::parse_operation,
};

/// With `merge_across_waves`, entity calls to the same subgraph that can run in the
/// same wave go in one request: fewer requests, no extra wave. With entity reuse on, the
/// inventory call for `topProducts` stays where it is, the later inventory call copies from it.
#[test]
fn merge_across_waves_sends_fewer_requests() -> Result<(), Box<dyn std::error::Error>> {
    use crate::query_planner::{
        graph::edge::PlannerOverrideContext,
        planner::{
            plan_nodes::planning::{PlanNode, QueryPlan},
            QueryPlannerOptions,
        },
        tests::testkit::build_query_plan,
    };
    init_logger();
    let plan_with = |merge_across_waves, entity_reuse| {
        build_query_plan(
            "../../bench/supergraph.graphql",
            parse_operation(&std::fs::read_to_string("../../bench/operation.graphql").unwrap()),
            PlannerOverrideContext::default(),
            QueryPlannerOptions {
                merge_across_waves,
                entity_reuse,
                ..Default::default()
            },
        )
    };
    let plan = |merge_across_waves| plan_with(merge_across_waves, false);
    // requests are fetches and batches, waves the steps of the top-level sequence
    let shape = |plan: &QueryPlan| {
        let text = plan.to_string();
        let requests = text
            .lines()
            .filter(|line| {
                let line = line.trim_start();
                line.starts_with("Fetch(") || line.starts_with("BatchFetch(")
            })
            .count();
        let waves = match plan.node() {
            Some(PlanNode::Sequence(sequence)) => sequence.nodes.len(),
            Some(_) => 1,
            None => 0,
        };
        (requests, waves)
    };
    let (requests, waves) = shape(&plan(false)?);
    let (merged_requests, merged_waves) = shape(&plan(true)?);
    assert_eq!(merged_waves, waves);
    assert!(
        merged_requests < requests,
        "{merged_requests} requests merged, {requests} without"
    );
    // merging there would leave the later call nothing to copy from
    assert_eq!(
        plan_with(true, true)?.to_string(),
        plan_with(false, true)?.to_string()
    );
    Ok(())
}

#[test]
fn test_bench_operation() -> Result<(), Box<dyn std::error::Error>> {
    init_logger();
    let document = parse_operation(
        &std::fs::read_to_string("../../bench/operation.graphql")
            .expect("Unable to read input file"),
    );
    let query_plan = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(format!("{}", query_plan), @r#"
    QueryPlan {
      Sequence {
        Parallel {
          Fetch(service: "products") {
            {
              topProducts {
                __typename
                upc
                name
                price
                weight
              }
            }
          },
          Fetch(service: "accounts") {
            {
              users {
                __typename
                id
                username
                name
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
                  ...a
                }
              }
            }
            _e1: _entities(representations: $__batch_reps_1) {
              ... on User {
                reviews {
                  id
                  body
                  product {
                    __typename
                    upc
                    reviews {
                      ...a
                    }
                  }
                }
              }
            }
          }
          fragment a on Review {
            id
            body
            author {
              __typename
              id
              username
              reviews {
                id
                body
                product {
                  __typename
                  upc
                }
              }
            }
          }
        },
        Parallel {
          BatchFetch(service: "products") {
            {
              _e0 {
                paths: [
                  "topProducts.@.reviews.@.author.reviews.@.product"
                  "users.@.reviews.@.product.reviews.@.author.reviews.@.product"
                  "users.@.reviews.@.product"
                ]
                {
                  ... on Product {
                    __typename
                    upc
                  }
                }
              }
            }
            {
              _e0: _entities(representations: $__batch_reps_0) {
                ... on Product {
                  name
                  price
                  weight
                }
              }
            }
          },
          BatchFetch(service: "accounts") {
            {
              _e0 {
                paths: [
                  "topProducts.@.reviews.@.author"
                  "users.@.reviews.@.product.reviews.@.author"
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
                ... on User {
                  name
                }
              }
            }
          },
        },
        BatchFetch(service: "inventory") {
          {
            _e0 {
              paths: [
                "topProducts.@.reviews.@.author.reviews.@.product"
                "users.@.reviews.@.product.reviews.@.author.reviews.@.product"
                "users.@.reviews.@.product"
                "topProducts.@"
              ]
              {
                ... on Product {
                  __typename
                  upc
                  price
                  weight
                }
              }
            }
          }
          {
            _e0: _entities(representations: $__batch_reps_0) {
              ... on Product {
                inStock
                shippingEstimate
              }
            }
          }
        },
      },
    },
    "#);

    Ok(())
}

/// Converts the benchmark plan to its executable form. The serialized output is what other tools
/// depend on, so it must stay the same after the planning data is dropped.
#[test]
fn preparing_a_plan_preserves_its_wire_form() -> Result<(), Box<dyn std::error::Error>> {
    init_logger();
    let document = parse_operation(&std::fs::read_to_string("../../bench/operation.graphql")?);
    let planning = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;
    let before = serde_json::to_string(&planning)?;
    let executable =
        planning.into_executable(&read_supergraph_state("../../bench/supergraph.graphql"));
    assert_eq!(serde_json::to_string(&executable)?, before);
    Ok(())
}
