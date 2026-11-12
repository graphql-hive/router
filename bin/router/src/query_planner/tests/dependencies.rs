use std::error::Error;

use crate::query_planner::{
    planner::plan_nodes::{dependencies::plan_fetches, planning::PlanNode},
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
        .map(|fetch| {
            (
                name(fetch.node),
                fetch
                    .waits_for
                    .iter()
                    .map(|dep| name(fetches[*dep].node))
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

/// A fetch that doesn't say what it depends on, like one a plugin built, waits for everything the
/// plan's order puts before it.
#[test]
fn a_fetch_without_dependencies_waits_for_the_plan_order() -> Result<(), Box<dyn Error>> {
    fn forget(node: &mut PlanNode, service: &str) {
        match node {
            PlanNode::Sequence(sequence) => {
                sequence.nodes.iter_mut().for_each(|n| forget(n, service))
            }
            PlanNode::Parallel(parallel) => {
                parallel.nodes.iter_mut().for_each(|n| forget(n, service))
            }
            PlanNode::Flatten(flatten) => forget(&mut flatten.node, service),
            PlanNode::Fetch(fetch) if fetch.service_name == service => fetch.depends_on = None,
            _ => {}
        }
    }
    init_logger();
    let mut plan = build_query_plan_with_defaults(
        "fixture/products-example.supergraph.graphql",
        parse_operation("{ topProducts { upc inStock reviews { author { name } } } }"),
    )?;
    forget(plan.node.as_mut().ok_or("an empty plan")?, "accounts");
    let fetches = plan_fetches(plan.node.as_ref().ok_or("an empty plan")?);
    assert_eq!(fetches[3].waits_for, [0, 1, 2]);
    Ok(())
}
