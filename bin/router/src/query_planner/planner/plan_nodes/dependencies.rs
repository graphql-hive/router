//! Which fetches of a plan wait for which, what the executor schedules on.
//!
//! A plan lists its fetches in `Sequence` and `Parallel` nodes, in waves: every fetch of a wave
//! waits for every fetch of the waves before it. Fetches built by the planner also say which
//! fetches they really read from (`depends_on`), so a fetch can start as soon as those are done,
//! not when its whole wave can.

use super::{PlanNode, PlanState, QueryPlan};

/// A fetch of a plan: any node that isn't a `Sequence` or a `Parallel`.
pub struct PlanFetch<'p, S: PlanState> {
    pub node: &'p PlanNode<S>,
    /// The fetches it waits for, by position in the list. Always earlier ones.
    pub waits_for: Vec<usize>,
}

/// The fetches of `node` in the plan's order, each with what it waits for: the fetches its
/// `depends_on` names, or, when it doesn't say, every fetch the plan's order puts before it.
pub fn plan_fetches<S: PlanState>(node: &PlanNode<S>) -> Vec<PlanFetch<'_, S>> {
    let mut fetches = Vec::new();
    collect(node, &[], &mut fetches, &mut |_, _| {});
    fetches
}

/// Checks that every fetch of a plan the planner built says what it depends on, and that each id
/// is a fetch (or one merged into a batch) that the plan's order puts before it. Returns the
/// first fetch that breaks it.
pub fn check_dependencies<S: PlanState>(plan: &QueryPlan<S>) -> Result<(), String> {
    let Some(node) = &plan.node else {
        return Ok(());
    };
    let mut problem = None;
    collect(node, &[], &mut Vec::new(), &mut |node, unresolved| {
        if problem.is_none() {
            problem = Some(match unresolved {
                None => format!("fetch {:?} doesn't say what it depends on", fetch_id(node)),
                Some(id) => format!(
                    "fetch {:?} depends on {id}, which isn't a fetch before it",
                    fetch_id(node)
                ),
            });
        }
    });
    problem.map_or(Ok(()), Err)
}

/// Adds the fetches in `node` to `fetches`, each after `preds`, and returns their positions.
/// `on_fallback` hears about a fetch that has to fall back to the plan's order, with the id that
/// isn't before it, or `None` when it doesn't say what it depends on.
fn collect<'p, S: PlanState>(
    node: &'p PlanNode<S>,
    preds: &[usize],
    fetches: &mut Vec<PlanFetch<'p, S>>,
    on_fallback: &mut impl FnMut(&PlanNode<S>, Option<i64>),
) -> Vec<usize> {
    match node {
        PlanNode::Sequence(sequence) => {
            let mut before = preds.to_vec();
            let mut all = Vec::new();
            for child in &sequence.nodes {
                let added = collect(child, &before, fetches, on_fallback);
                before.extend(&added);
                all.extend(added);
            }
            all
        }
        PlanNode::Parallel(parallel) => parallel
            .nodes
            .iter()
            .flat_map(|child| collect(child, preds, fetches, on_fallback))
            .collect(),
        node => {
            let waits_for = match depends_on(node) {
                Some(ids) => match resolve(ids, preds, fetches) {
                    Ok(waits_for) => waits_for,
                    Err(id) => {
                        on_fallback(node, Some(id));
                        preds.to_vec()
                    }
                },
                None => {
                    on_fallback(node, None);
                    preds.to_vec()
                }
            };
            fetches.push(PlanFetch { node, waits_for });
            vec![fetches.len() - 1]
        }
    }
}

/// The fetches among `preds` that have the `ids`, or the first id that none of them has.
fn resolve<S: PlanState>(
    ids: &[i64],
    preds: &[usize],
    fetches: &[PlanFetch<S>],
) -> Result<Vec<usize>, i64> {
    let mut waits_for = ids
        .iter()
        .map(|id| {
            preds
                .iter()
                .copied()
                .find(|pred| fetch_ids(fetches[*pred].node).any(|pred_id| pred_id == *id))
                .ok_or(*id)
        })
        .collect::<Result<Vec<_>, _>>()?;
    waits_for.sort_unstable();
    waits_for.dedup();
    Ok(waits_for)
}

/// What the fetch in `node` depends on, `None` when it doesn't say.
fn depends_on<S: PlanState>(node: &PlanNode<S>) -> Option<&[i64]> {
    match node {
        PlanNode::Fetch(fetch) => fetch.depends_on.as_deref(),
        PlanNode::Flatten(flatten) => depends_on(&flatten.node),
        PlanNode::BatchFetch(batch) => batch.depends_on.as_deref(),
        PlanNode::Subscription(subscription) => subscription.primary.depends_on.as_deref(),
        _ => None,
    }
}

fn fetch_id<S: PlanState>(node: &PlanNode<S>) -> Option<i64> {
    match node {
        PlanNode::Fetch(fetch) => Some(fetch.id),
        PlanNode::Flatten(flatten) => fetch_id(&flatten.node),
        PlanNode::BatchFetch(batch) => Some(batch.id),
        PlanNode::Subscription(subscription) => Some(subscription.primary.id),
        _ => None,
    }
}

/// The ids of the fetches in `node`, a batch has all the ones merged into it.
fn fetch_ids<S: PlanState>(node: &PlanNode<S>) -> impl Iterator<Item = i64> + '_ {
    let completes: &[i64] = match node {
        PlanNode::BatchFetch(batch) => &batch.completes,
        _ => &[],
    };
    fetch_id(node).into_iter().chain(completes.iter().copied())
}
