//! Checks the plan the planner picks against every plan the walker's paths make, to find cases
//! where a better one exists. The walker keeps every path it finds for this, not only the best
//! ones, and each pick of a path per leaf goes through the real fetch graph, so the fetches and
//! waves are the ones the plan really has.
//!
//! It only sees plans made of paths the walker finds and fetch graphs the fetch graph builds, so a
//! better plan that needs something else isn't found.
//!
//! `PLAN_ORACLE=cases.tsv cargo nextest run -p hive-router plan_oracle --run-ignored only`
//! Each line of the file is `name<TAB>supergraph path<TAB>query`. Results go to `<file>.results`,
//! the plans of every miss to `<file>.misses`. Nextest stops a test after 30s, so a big file goes
//! in chunks.

use std::error::Error;
use std::fmt::Write as _;

use crate::query_planner::{
    ast::normalization::normalize_operation,
    graph::{edge::PlannerOverrideContext, Graph},
    planner::{
        add_variables_to_fetch_steps,
        best::{every_combination, find_best_combination},
        fetch::fetch_graph::build_fetch_graph_from_query_tree,
        plan_nodes::planning::{PlanNode, QueryPlan},
        query_plan::build_query_plan_from_fetch_graph,
        tree::query_tree::QueryTree,
        walker::{walk_operation, KEEP_EVERY_PATH},
        QueryPlannerOptions,
    },
    state::supergraph_state::SupergraphState,
    utils::{
        cancellation::CancellationToken,
        parsing::{parse_operation, parse_schema},
    },
};

/// Past this many plans for one query it's skipped, trying them all would take too long.
const LIMIT: usize = 20_000;

#[test]
#[ignore = "needs PLAN_ORACLE, see the module docs"]
fn plan_oracle() -> Result<(), Box<dyn Error>> {
    let cases_path = std::env::var("PLAN_ORACLE")?;
    let mut results = String::from(
        "name\tpicked\tbest\tplans\tfailed\tverdict\tpicked from every path\tbest by waves\tverdict by waves\n",
    );
    let mut misses = String::new();
    let (mut checked, mut missed, mut skipped, mut broken) = (0, 0, 0, 0);

    // panics are counted, not printed
    let hook = std::panic::take_hook();
    if std::env::var_os("PLAN_ORACLE_LOUD").is_none() {
        std::panic::set_hook(Box::new(|_| {}));
    }
    for line in std::fs::read_to_string(&cases_path)?.lines() {
        let [name, supergraph, query] = line.splitn(3, '\t').collect::<Vec<_>>()[..] else {
            continue;
        };
        KEEP_EVERY_PATH.set(false);
        let checked_case =
            std::panic::catch_unwind(|| check(supergraph, query)).unwrap_or_else(|panic| {
                Err(format!("planner panicked: {}", panic_message(&panic)).into())
            });
        match checked_case {
            Err(error) => {
                broken += 1;
                writeln!(results, "{name}\t\t\t\t\terror: {error}")?;
            }
            Ok(Check::TooMany { picked }) => {
                skipped += 1;
                writeln!(results, "{name}\t{picked:?}\t\t>{LIMIT}\t\tskipped")?;
            }
            Ok(Check::Done {
                picked,
                from_every_path,
                picked_plan,
                best,
                best_plan,
                best_by_waves,
                best_by_waves_plan,
                plans,
                failed,
            }) => {
                checked += 1;
                let mut verdict = |best: (usize, usize),
                                   best_plan: &str,
                                   rank: fn((usize, usize)) -> (usize, usize),
                                   by: &str|
                 -> Result<String, std::fmt::Error> {
                    if rank(best) >= rank(picked) {
                        return Ok("ok".to_string());
                    }
                    // the walker kept the path the best plan needs, or it didn't
                    let cause = if rank(from_every_path) <= rank(best) {
                        "walker"
                    } else {
                        "choice"
                    };
                    writeln!(
                        misses,
                        "=== {name}: picked {picked:?}, best {best:?} by {by}, missed by the {cause}\n{query}\n\n--- picked\n{picked_plan}\n--- best\n{best_plan}\n"
                    )?;
                    Ok(format!("MISS {cause}"))
                };
                let by_fetches = verdict(best, &best_plan, |shape| shape, "fetches")?;
                let by_waves = verdict(
                    best_by_waves,
                    &best_by_waves_plan,
                    |(fetches, waves)| (waves, fetches),
                    "waves",
                )?;
                if by_fetches != "ok" || by_waves != "ok" {
                    missed += 1;
                }
                writeln!(
                    results,
                    "{name}\t{picked:?}\t{best:?}\t{plans}\t{failed}\t{by_fetches}\t{from_every_path:?}\t{best_by_waves:?}\t{by_waves}"
                )?;
            }
        }
    }

    std::panic::set_hook(hook);
    std::fs::write(format!("{cases_path}.results"), results)?;
    std::fs::write(format!("{cases_path}.misses"), misses)?;
    println!("checked {checked}, missed {missed}, skipped {skipped}, broken {broken}");
    Ok(())
}

enum Check {
    TooMany {
        picked: (usize, usize),
    },
    Done {
        picked: (usize, usize),
        /// What the passes pick when the walker keeps every path.
        from_every_path: (usize, usize),
        picked_plan: String,
        /// The fewest fetches, then the fewest waves, among all plans.
        best: (usize, usize),
        best_plan: String,
        /// The fewest waves, then the fewest fetches, as (fetches, waves) too.
        best_by_waves: (usize, usize),
        best_by_waves_plan: String,
        plans: usize,
        /// Plans the fetch graph or the plan builder gave up on.
        failed: usize,
    },
}

fn check(supergraph_path: &str, query: &str) -> Result<Check, Box<dyn Error>> {
    let schema = parse_schema(&std::fs::read_to_string(supergraph_path)?);
    let state = SupergraphState::new(&schema);
    let graph = Graph::graph_from_supergraph_state(&state)?;
    let document = parse_operation(query);
    let normalized = normalize_operation(&state, &document, None)?;
    let operation = normalized.executable_operation();
    let overrides = PlannerOverrideContext::default();
    let token = CancellationToken::new();

    let plan_tree = |tree: QueryTree| -> Result<QueryPlan, Box<dyn Error>> {
        let options = QueryPlannerOptions::default();
        let mut fetch_graph = build_fetch_graph_from_query_tree(
            &graph, &state, &overrides, tree, operation, &options, &token,
        )?;
        add_variables_to_fetch_steps(&mut fetch_graph, &operation.variable_definitions)?;
        Ok(build_query_plan_from_fetch_graph(
            fetch_graph,
            &state,
            &options,
            &token,
        )?)
    };

    let paths = walk_operation(&graph, &state, &overrides, operation, &token)?;
    let picked_plan = plan_tree(find_best_combination(&graph, paths, &token)?)?;
    let picked = fetches_and_waves(&picked_plan);
    // only the picked plan, for when the rest would take too long
    if std::env::var_os("PLAN_ORACLE_ONLY_PICKED").is_some() {
        return Ok(Check::TooMany { picked });
    }

    KEEP_EVERY_PATH.set(true);
    let every_path = walk_operation(&graph, &state, &overrides, operation, &token);
    let every_path_again = walk_operation(&graph, &state, &overrides, operation, &token);
    KEEP_EVERY_PATH.set(false);
    let from_every_path = fetches_and_waves(&plan_tree(find_best_combination(
        &graph,
        every_path_again?,
        &token,
    )?)?);

    let mut best: Option<((usize, usize), QueryPlan)> = None;
    let mut best_by_waves: Option<((usize, usize), QueryPlan)> = None;
    let mut failed = 0;
    let plans = every_combination(
        &graph,
        every_path?,
        LIMIT,
        |tree| match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plan_tree(tree)))
            .unwrap_or_else(|_| Err("panicked".into()))
        {
            Ok(plan) => {
                let shape = fetches_and_waves(&plan);
                let by_waves = |(fetches, waves): (usize, usize)| (waves, fetches);
                if best_by_waves
                    .as_ref()
                    .is_none_or(|(best, _)| by_waves(shape) < by_waves(*best))
                {
                    best_by_waves = Some((shape, plan.clone()));
                }
                if best.as_ref().is_none_or(|(best, _)| shape < *best) {
                    best = Some((shape, plan));
                }
            }
            Err(_) => failed += 1,
        },
    )?;
    let Some(plans) = plans else {
        return Ok(Check::TooMany { picked });
    };
    let (best, best_plan) = best.ok_or("no plan worked")?;
    let (best_by_waves, best_by_waves_plan) = best_by_waves.ok_or("no plan worked")?;
    Ok(Check::Done {
        picked,
        from_every_path,
        picked_plan: picked_plan.to_string(),
        best,
        best_plan: best_plan.to_string(),
        best_by_waves,
        best_by_waves_plan: best_by_waves_plan.to_string(),
        plans,
        failed,
    })
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default()
        .replace('\n', " ")
}

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
