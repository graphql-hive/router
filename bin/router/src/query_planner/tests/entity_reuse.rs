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

/// One line per fetch (a batch, one per alias): its id, where it merges, where it may copy from,
/// and the fetches it waits for. A root fetch merges at `root`. A batch lists every fetch it
/// completes, so each id in a wait list is one of the lines.
fn describe(plan: &QueryPlan) -> String {
    fn walk(node: &PlanNode, out: &mut String) {
        match node {
            PlanNode::Sequence(s) => s.nodes.iter().for_each(|n| walk(n, out)),
            PlanNode::Parallel(p) => p.nodes.iter().for_each(|n| walk(n, out)),
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
        walk(node, &mut out);
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
    [13] products at root <- - after []
    [2] accounts at root <- - after []
    [15] inventory at topProducts.@ <- - after [13]
    [3, 17] reviews at topProducts.@ <- - after [2, 13]
    [3, 17] reviews at users.@ <- - after [2, 13]
    [7, 12, 22] products at topProducts.@.reviews.@.author.reviews.@.product + users.@.reviews.@.product.reviews.@.author.reviews.@.product + users.@.reviews.@.product <- topProducts.@ after [3, 17]
    [8, 18] accounts at topProducts.@.reviews.@.author + users.@.reviews.@.product.reviews.@.author <- users.@ after [3, 17]
    [4, 9, 19] inventory at topProducts.@.reviews.@.author.reviews.@.product + users.@.reviews.@.product.reviews.@.author.reviews.@.product + users.@.reviews.@.product <- topProducts.@ after [7, 12, 15, 22]
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
