//! Supergraphs where an easy-to-make plan isn't the best one. Each test asserts the best plan's
//! fetches and waves, and says what used to miss it. A case we miss again goes `#[ignore]` with
//! the reason, `cargo nextest run --run-ignored all plan_quality` shows where we stand.
//!
//! Two parts of the planner can miss a plan:
//! - The walker keeps a leaf's paths that cost less than one more subgraph hop than its
//!   cheapest, worked out for that leaf alone. What it drops, nothing after it can pick.
//! - The search (`find_best_combination`) picks among those paths, greedy a leaf at a time,
//!   plus a second pick that splits each hop's cost between the leaves that could share it.

use std::error::Error;

use crate::query_planner::{
    planner::plan_nodes::planning::{PlanNode, QueryPlan},
    tests::testkit::{build_query_plan_with_defaults, init_logger},
    utils::parsing::parse_operation,
};

/// How many fetches the plan makes, and how many rounds of them run one after another.
fn fetches_and_waves(plan: &QueryPlan) -> (usize, usize) {
    plan.node().map_or((0, 0), shape)
}

fn shape(node: &PlanNode) -> (usize, usize) {
    let in_sequence =
        |(fetches, waves), (more_fetches, more_waves)| (fetches + more_fetches, waves + more_waves);
    let in_parallel = |(fetches, waves): (usize, usize), (more_fetches, more_waves)| {
        (fetches + more_fetches, waves.max(more_waves))
    };
    match node {
        PlanNode::Fetch(_) | PlanNode::BatchFetch(_) | PlanNode::Subscription(_) => (1, 1),
        PlanNode::Flatten(flatten) => shape(&flatten.node),
        PlanNode::Sequence(sequence) => sequence.nodes.iter().map(shape).fold((0, 0), in_sequence),
        PlanNode::Parallel(parallel) => parallel.nodes.iter().map(shape).fold((0, 0), in_parallel),
        PlanNode::Defer(defer) => {
            let primary = defer.primary.node.as_deref().map_or((0, 0), shape);
            let deferred = defer
                .deferred
                .iter()
                .filter_map(|deferred| deferred.node.as_deref())
                .map(shape)
                .fold((0, 0), in_parallel);
            in_sequence(primary, deferred)
        }
    }
}

fn plan(fixture: &str, query: &str) -> Result<(usize, usize), Box<dyn Error>> {
    init_logger();
    let plan = build_query_plan_with_defaults(
        &format!("fixture/tests/plan-quality-{fixture}.supergraph.graphql"),
        parse_operation(query),
    )?;
    Ok(fetches_and_waves(&plan))
}

// `c` has every field, `a` and `b` half each, and all keys are the same size. Every field ties
// between its half and `c`. Greedy goes a field at a time and opens `a` for the a-fields and `b`
// for the b-fields: 3 fetches, where `c` alone is 2. Priced by shares, `c` serves four fields.
#[test]
fn cover_all_tie() -> Result<(), Box<dyn Error>> {
    assert_eq!(plan("cover-all-tie", "{ t { a1 a2 b1 b2 } }")?, (2, 2));
    assert_eq!(plan("cover-all-tie", "{ t { b2 b1 a2 a1 } }")?, (2, 2));
    Ok(())
}

// The same, but `c`'s key is a field bigger, so `a` or `b` is a bit cheaper for every field on
// its own. The walker has to keep `c` as a near-tie for the shares pick to find it.
#[test]
fn cover_all() -> Result<(), Box<dyn Error>> {
    assert_eq!(plan("cover-all", "{ t { a1 a2 b1 b2 } }")?, (2, 2));
    assert_eq!(plan("cover-all", "{ t { b2 b1 a2 a1 } }")?, (2, 2));
    Ok(())
}

// `c1` only comes from `c`, so `c` is in the plan anyway. `a1` could come from it too, but `a`
// is a bit cheaper for `a1` on its own (`c`'s key is bigger). Keeping only that one opened `a`.
#[test]
fn needed_anyway() -> Result<(), Box<dyn Error>> {
    assert_eq!(plan("needed-anyway", "{ t { a1 c1 } }")?, (2, 2));
    Ok(())
}

// The same with equal keys: both paths reach the search, and `c1` has one way only, so `c` is
// in the plan before `a1` picks.
#[test]
fn needed_anyway_tie() -> Result<(), Box<dyn Error>> {
    assert_eq!(plan("needed-anyway-tie", "{ t { a1 c1 } }")?, (2, 2));
    Ok(())
}

// `b`'s key `ref` only comes from `a`, so going to `b` takes a hop to `a` first. Counting only
// the key's size, root -> `b` looked cheaper than root -> `c`: 3 waves where `c` does it in 2.
#[test]
fn extra_wave() -> Result<(), Box<dyn Error>> {
    assert_eq!(plan("extra-wave", "{ t { z } }")?, (2, 2));
    Ok(())
}

// With `a` in the query too, `z` goes to `c`, but `a` is a bit cheaper from `a` on its own
// (`c`'s key is bigger). Keeping only that one opened both.
#[test]
fn extra_wave_with_a() -> Result<(), Box<dyn Error>> {
    assert_eq!(plan("extra-wave", "{ t { a z } }")?, (2, 2));
    assert_eq!(plan("extra-wave", "{ t { z a } }")?, (2, 2));
    Ok(())
}

// `f` in `b` @requires `v { v }` from `c`. Entering `b` with `k1` (from root) looks as cheap as
// with `k2` (from `c`), but then `c` needs `k2`, which only `b` has: `b` for `k2`, `c`, then `b`
// again for `f`. Going to `c` first with `k1`, it returns `k2` and `v` for one call to `b`.
// Apollo's planner makes the 4 fetches here.
#[test]
fn requires_key_from_own_subgraph() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        plan("requires-key-from-own-subgraph", "{ u { f } }")?,
        (3, 3)
    );
    Ok(())
}

// `x` in `c` @requires `w`, which only `d` has. Counting only the requirement's size, the path
// through `c` looked cheaper than `a` (bigger key): root -> `d` -> `c` where root -> `a` is enough.
#[test]
fn requires_hidden_hop() -> Result<(), Box<dyn Error>> {
    assert_eq!(plan("requires-hidden-hop", "{ t { x } }")?, (2, 2));
    Ok(())
}
