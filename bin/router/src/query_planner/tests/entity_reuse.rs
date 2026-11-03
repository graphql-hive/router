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

#[test]
fn source_with_an_aliased_key_is_left_out() -> Result<(), Box<dyn Error>> {
    init_logger();
    // `id` on users is the username, the executor would match authors by it
    let document =
        parse_operation(r#"{ users { id: username name reviews { author { name } } } }"#);
    let query_plan = build_query_plan_with_defaults("../../bench/supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    reviews at users.@ <- -
    accounts at users.@.reviews.@.author <- -
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
    let query_plan =
        build_query_plan_with_defaults("fixture/tests/mutations.supergraph.graphql", document)?;

    insta::assert_snapshot!(describe(&query_plan), @r"
    b at first <- -
    b at second <- -
    ");

    Ok(())
}
