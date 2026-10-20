use crate::query_planner::{
    planner::plan_nodes::planning::{PlanNode, QueryPlan},
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};
use std::error::Error;

/// One line per entity fetch (or batch alias): where it merges, and where it may copy from.
fn describe(plan: &QueryPlan) -> String {
    fn walk(node: &PlanNode, out: &mut String) {
        match node {
            PlanNode::Sequence(s) => s.nodes.iter().for_each(|n| walk(n, out)),
            PlanNode::Parallel(p) => p.nodes.iter().for_each(|n| walk(n, out)),
            PlanNode::Condition(c) => c
                .if_clause
                .iter()
                .chain(c.else_clause.iter())
                .for_each(|n| walk(n, out)),
            PlanNode::Flatten(flatten) => {
                if let PlanNode::Fetch(fetch) = flatten.node.as_ref() {
                    line(
                        out,
                        &fetch.service_name,
                        &flatten.path.to_string(),
                        fetch.entity_reuse.as_deref(),
                    );
                }
            }
            PlanNode::BatchFetch(batch) => {
                for alias in &batch.entity_batch.aliases {
                    let paths: Vec<String> =
                        alias.merge_paths.iter().map(|p| p.to_string()).collect();
                    line(
                        out,
                        &batch.service_name,
                        &paths.join(" + "),
                        alias.entity_reuse.as_deref(),
                    );
                }
            }
            _ => {}
        }
    }
    fn line(
        out: &mut String,
        service: &str,
        path: &str,
        reuse: Option<&crate::query_planner::planner::plan_nodes::EntityReuse>,
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
        out.push_str(&format!("{service} at {path} <- {sources}\n"));
    }
    let mut out = String::new();
    if let Some(node) = &plan.node {
        walk(node, &mut out);
    }
    out
}

#[test]
fn bench_operation_entities_can_come_from_earlier_fetches() -> Result<(), Box<dyn Error>> {
    init_logger();
    let document = parse_operation(
        &std::fs::read_to_string("../../bench/operation.graphql")
            .expect("Unable to read input file"),
    );
    let query_plan = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    inventory at topProducts.@ <- -
    reviews at topProducts.@ <- -
    reviews at users.@ <- -
    products at topProducts.@.reviews.@.author.reviews.@.product + users.@.reviews.@.product + users.@.reviews.@.product.reviews.@.author.reviews.@.product <- topProducts.@
    accounts at topProducts.@.reviews.@.author + users.@.reviews.@.product.reviews.@.author <- users.@
    inventory at topProducts.@.reviews.@.author.reviews.@.product + users.@.reviews.@.product + users.@.reviews.@.product.reviews.@.author.reviews.@.product <- topProducts.@
    ");

    Ok(())
}
