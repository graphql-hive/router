use std::error::Error;

use crate::query_planner::{
    planner::plan_nodes::{
        dependencies::plan_fetches, planning::PlanNode, FetchNode, Planning, QueryPlan,
    },
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};

/// Each fetch as `service`, with the services of the fetches it waits for.
type Waits = Vec<(String, Vec<String>)>;

fn waits(fixture: &str, query: &str) -> Result<Waits, Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(fixture, parse_operation(query))?;
    let fetches = plan_fetches(plan.node().ok_or("an empty plan")?);
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
    // the schedule keeps who waits for each fetch, turn it around
    let mut waits: Waits = fetches.iter().map(|node| (name(node), vec![])).collect();
    for (dep, node) in fetches.iter().enumerate() {
        for dependent in plan.schedule().dependents(dep) {
            waits[*dependent as usize].1.push(name(node));
        }
    }
    Ok(waits)
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

fn fetch_mut<'p>(node: &'p mut PlanNode, service: &str) -> Option<&'p mut FetchNode<Planning>> {
    match node {
        PlanNode::Sequence(sequence) => sequence
            .nodes
            .iter_mut()
            .find_map(|node| fetch_mut(node, service)),
        PlanNode::Parallel(parallel) => parallel
            .nodes
            .iter_mut()
            .find_map(|node| fetch_mut(node, service)),
        PlanNode::Flatten(flatten) => fetch_mut(&mut flatten.node, service),
        PlanNode::Fetch(fetch) if fetch.service_name == service => Some(fetch),
        _ => None,
    }
}

/// A fetch that depends on one that comes after it can't be scheduled, so the plan isn't made.
#[test]
fn a_plan_with_a_dependency_that_comes_later_is_refused() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        "fixture/products-example.supergraph.graphql",
        parse_operation("{ topProducts { upc inStock reviews { author { name } } } }"),
    )?;
    let mut node = plan.into_node().ok_or("an empty plan")?;
    // the authors come last, the reviews can't wait for them
    let accounts = fetch_mut(&mut node, "accounts")
        .ok_or("an accounts fetch")?
        .id;
    fetch_mut(&mut node, "reviews")
        .ok_or("a reviews fetch")?
        .depends_on = smallvec::smallvec![accounts];
    let error = QueryPlan::new(Some(node)).expect_err("a plan that can't be scheduled");
    assert!(
        error
            .to_string()
            .contains(&format!("depends on fetch {accounts},")),
        "{error}"
    );
    Ok(())
}

/// Two fetches with one id can't be told apart in `depends_on`, so the plan isn't made.
#[test]
fn a_plan_with_two_fetches_with_one_id_is_refused() -> Result<(), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        "fixture/products-example.supergraph.graphql",
        parse_operation("{ topProducts { upc inStock reviews { author { name } } } }"),
    )?;
    let mut node = plan.into_node().ok_or("an empty plan")?;
    let reviews = fetch_mut(&mut node, "reviews").ok_or("a reviews fetch")?.id;
    // the authors wait for the reviews, and would wait for the inventory call instead
    fetch_mut(&mut node, "inventory")
        .ok_or("an inventory fetch")?
        .id = reviews;
    let error = QueryPlan::new(Some(node)).expect_err("a plan with a duplicate id");
    assert!(
        error
            .to_string()
            .contains(&format!("Two fetches have id {reviews}")),
        "{error}"
    );
    Ok(())
}
