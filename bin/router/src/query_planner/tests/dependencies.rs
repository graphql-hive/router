use std::error::Error;

use crate::query_planner::{
    planner::plan_nodes::{dependencies::plan_fetches, planning::PlanNode, QueryPlan},
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};

/// Each fetch as `service`, with the services of the fetches it waits for.
type Waits = Vec<(String, Vec<String>)>;

fn waits(fixture: &str, query: &str) -> Result<Waits, Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(fixture, parse_operation(query))?;
    let fetches = plan_fetches(plan.node.as_ref().ok_or("an empty plan")?);
    let name = |node: &PlanNode| -> String {
        match node {
            PlanNode::Fetch(fetch) => fetch.service_name.clone(),
            PlanNode::Flatten(flatten) => match flatten.node.as_ref() {
                PlanNode::Fetch(fetch) => fetch.service_name.clone(),
                other => format!("{other:?}"),
            },
            PlanNode::BatchFetch(batch) => format!("batch:{}", batch.service_name),
            other => format!("{other:?}"),
        }
    };
    Ok(fetches
        .iter()
        .enumerate()
        .map(|(index, node)| {
            (
                name(node),
                plan.schedule()
                    .waits_for(index)
                    .iter()
                    .map(|dep| name(fetches[*dep]))
                    .collect(),
            )
        })
        .collect())
}

/// The plan has `inventory` and `reviews` in one wave and `accounts` in the next, but the
/// authors only need the reviews.
#[test]
fn a_fetch_depends_only_on_what_it_reads() -> Result<(), Box<dyn Error>> {
    let waits = waits(
        "fixture/products-example.supergraph.graphql",
        "{ topProducts { upc inStock reviews { author { name } } } }",
    )?;
    assert_eq!(
        waits,
        [
            ("products".to_string(), vec![]),
            ("reviews".to_string(), vec!["products".to_string()]),
            ("inventory".to_string(), vec!["products".to_string()]),
            ("accounts".to_string(), vec!["reviews".to_string()]),
        ]
    );
    Ok(())
}

/// Root mutation fields run one after another, entity calls included: `add` waits for the
/// `isExpensive` call of `addProduct`, and `delete` for `add`.
#[test]
fn mutation_fields_depend_on_the_one_before() -> Result<(), Box<dyn Error>> {
    let waits = waits(
        "fixture/tests/mutations.supergraph.graphql",
        r#"mutation {
          create: addProduct(input: { name: "new", price: 599.99 }) { isExpensive }
          count: add(num: 1)
          deleted: delete
        }"#,
    )?;
    assert_eq!(
        waits,
        [
            ("a".to_string(), vec![]),
            ("b".to_string(), vec!["a".to_string()]),
            ("c".to_string(), vec!["b".to_string()]),
            ("b".to_string(), vec!["c".to_string()]),
        ]
    );
    Ok(())
}

/// A fetch that depends on one the plan's order doesn't put before it can't be scheduled, so
/// the plan isn't made.
#[test]
fn a_plan_with_a_dependency_that_is_not_before_it_is_refused() -> Result<(), Box<dyn Error>> {
    fn depend_on(node: &mut PlanNode, service: &str, id: u32) {
        match node {
            PlanNode::Sequence(sequence) => sequence
                .nodes
                .iter_mut()
                .for_each(|node| depend_on(node, service, id)),
            PlanNode::Parallel(parallel) => parallel
                .nodes
                .iter_mut()
                .for_each(|node| depend_on(node, service, id)),
            PlanNode::Flatten(flatten) => depend_on(&mut flatten.node, service, id),
            PlanNode::Fetch(fetch) if fetch.service_name == service => {
                fetch.depends_on = vec![id].into_boxed_slice()
            }
            _ => {}
        }
    }
    init_logger();
    let plan = build_query_plan_with_defaults(
        "fixture/products-example.supergraph.graphql",
        parse_operation("{ topProducts { upc inStock reviews { author { name } } } }"),
    )?;
    let mut node = plan.node.ok_or("an empty plan")?;
    // `inventory` and `reviews` run side by side, one can't wait for the other
    let reviews_id = match &node {
        PlanNode::Sequence(sequence) => match &sequence.nodes[1] {
            PlanNode::Parallel(parallel) => parallel
                .nodes
                .iter()
                .find_map(|node| match node {
                    PlanNode::Flatten(flatten) => match flatten.node.as_ref() {
                        PlanNode::Fetch(fetch) if fetch.service_name == "reviews" => Some(fetch.id),
                        _ => None,
                    },
                    _ => None,
                })
                .ok_or("a reviews fetch")?,
            _ => return Err("a parallel wave".into()),
        },
        _ => return Err("a sequence".into()),
    };
    depend_on(&mut node, "inventory", reviews_id);
    let error = QueryPlan::new(Some(node)).expect_err("a plan that can't be scheduled");
    assert!(
        error.contains(&format!("depends on {reviews_id}")),
        "{error}"
    );
    Ok(())
}
