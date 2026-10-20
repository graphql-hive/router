use crate::query_planner::{
    planner::plan_nodes::planning::{PlanNode, QueryPlan},
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

/// One line per fetch: what it is and which fetches it waits for.
fn describe(plan: &QueryPlan) -> String {
    let Some(schedule) = &plan.ready_schedule else {
        return "no schedule".to_string();
    };
    let Some(PlanNode::Sequence(sequence)) = &plan.node else {
        return "schedule on a non-Sequence plan".to_string();
    };

    fn label(node: &PlanNode) -> String {
        match node {
            PlanNode::Fetch(fetch) => format!("Fetch({})", fetch.service_name),
            PlanNode::Flatten(flatten) => match flatten.node.as_ref() {
                PlanNode::Fetch(fetch) => {
                    format!("Flatten({}, {})", flatten.path, fetch.service_name)
                }
                _ => "Flatten(?)".to_string(),
            },
            PlanNode::BatchFetch(batch) => format!(
                "BatchFetch({}, {} fetches)",
                batch.service_name,
                batch.fetch_node_ids.len()
            ),
            PlanNode::Condition(condition) => format!("Condition({})", condition.condition),
            _ => "?".to_string(),
        }
    }

    let mut out = String::new();
    for (index, &(wave, position)) in schedule.fetches.iter().enumerate() {
        let node = match &sequence.nodes[wave] {
            PlanNode::Parallel(parallel) => &parallel.nodes[position],
            node => node,
        };
        let parents: Vec<usize> = (0..schedule.fetches.len())
            .filter(|&parent| schedule.dependents[parent].contains(&index))
            .collect();
        assert_eq!(parents.len(), schedule.waiting_on[index]);
        out.push_str(&format!(
            "{index} (wave {wave}): {} waits on {parents:?}\n",
            label(node)
        ));
    }
    out
}

#[test]
fn bench_operation_fetches_wait_only_for_their_parents() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        &std::fs::read_to_string("../../bench/operation.graphql")
            .expect("Unable to read input file"),
    );
    let query_plan = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    0 (wave 0): Fetch(products) waits on []
    1 (wave 0): Fetch(accounts) waits on []
    2 (wave 1): Flatten(topProducts.@, inventory) waits on [0]
    3 (wave 1): BatchFetch(reviews, 2 fetches) waits on [0, 1]
    4 (wave 2): BatchFetch(products, 3 fetches) waits on [0, 3]
    5 (wave 2): BatchFetch(accounts, 2 fetches) waits on [1, 3]
    6 (wave 3): BatchFetch(inventory, 3 fetches) waits on [2, 4]
    ");

    Ok(())
}

#[test]
fn mutations_get_no_schedule() -> Result<(), Box<dyn Error>> {
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

    assert!(query_plan.ready_schedule.is_none());

    Ok(())
}
