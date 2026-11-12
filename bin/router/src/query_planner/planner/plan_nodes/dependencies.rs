//! Which fetches of a plan wait for which, what the executor schedules on.
//!
//! A plan lists its fetches in `Sequence` and `Parallel` nodes, in waves: every fetch of a wave
//! comes after every fetch of the waves before it. The planner also tells each fetch which fetches
//! it really reads from (`depends_on`), so a fetch can start as soon as those are done, not when
//! its whole wave could. The schedule is worked out once, with the plan.

use super::{PlanNode, PlanState};

/// What every fetch of a plan waits for. Fetches are counted in the plan's order, the order
/// [`plan_fetches`] lists them in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanSchedule {
    /// For each fetch, the fetches it waits for. Always earlier ones.
    waits_for: Box<[Box<[usize]>]>,
}

impl PlanSchedule {
    /// The schedule of `node`. Fails when a fetch depends on an id that isn't a fetch (or one
    /// merged into a batch) the plan's order puts before it.
    pub fn new<S: PlanState>(node: Option<&PlanNode<S>>) -> Result<Self, String> {
        let mut fetches = Vec::new();
        let mut waits_for = Vec::new();
        if let Some(node) = node {
            collect(node, &[], &mut fetches, &mut waits_for)?;
        }
        Ok(Self {
            waits_for: waits_for.into_boxed_slice(),
        })
    }

    /// How many fetches the plan has.
    pub fn len(&self) -> usize {
        self.waits_for.len()
    }

    pub fn is_empty(&self) -> bool {
        self.waits_for.is_empty()
    }

    /// The fetches the `index`th fetch waits for.
    pub fn waits_for(&self, index: usize) -> &[usize] {
        &self.waits_for[index]
    }

    /// The schedule of the plan without its first fetch, for the rest of a subscription's plan
    /// that runs on every event: the subscription fetch is done by then.
    pub fn without_first(&self) -> Self {
        Self {
            waits_for: self
                .waits_for
                .iter()
                .skip(1)
                .map(|waits_for| {
                    waits_for
                        .iter()
                        .filter(|index| **index != 0)
                        .map(|index| index - 1)
                        .collect()
                })
                .collect(),
        }
    }
}

/// The fetches of `node` in the plan's order: every node that isn't a `Sequence` or a `Parallel`.
pub fn plan_fetches<S: PlanState>(node: &PlanNode<S>) -> Vec<&PlanNode<S>> {
    let mut fetches = Vec::new();
    push_fetches(node, &mut fetches);
    fetches
}

fn push_fetches<'p, S: PlanState>(node: &'p PlanNode<S>, fetches: &mut Vec<&'p PlanNode<S>>) {
    match node {
        PlanNode::Sequence(sequence) => sequence
            .nodes
            .iter()
            .for_each(|node| push_fetches(node, fetches)),
        PlanNode::Parallel(parallel) => parallel
            .nodes
            .iter()
            .for_each(|node| push_fetches(node, fetches)),
        node => fetches.push(node),
    }
}

/// Adds the fetches in `node` to `fetches` and what they wait for to `waits_for`, and returns
/// their positions. `preds` are the fetches the plan's order puts before `node`.
fn collect<'p, S: PlanState>(
    node: &'p PlanNode<S>,
    preds: &[usize],
    fetches: &mut Vec<&'p PlanNode<S>>,
    waits_for: &mut Vec<Box<[usize]>>,
) -> Result<Vec<usize>, String> {
    match node {
        PlanNode::Sequence(sequence) => {
            let mut before = preds.to_vec();
            let mut all = Vec::new();
            for child in &sequence.nodes {
                let added = collect(child, &before, fetches, waits_for)?;
                before.extend(&added);
                all.extend(added);
            }
            Ok(all)
        }
        PlanNode::Parallel(parallel) => {
            let mut all = Vec::new();
            for child in &parallel.nodes {
                all.extend(collect(child, preds, fetches, waits_for)?);
            }
            Ok(all)
        }
        node => {
            let mut waits: Vec<usize> = depends_on(node)
                .iter()
                .map(|id| {
                    preds
                        .iter()
                        .copied()
                        .find(|pred| fetch_ids(fetches[*pred]).any(|pred_id| pred_id == *id))
                        .ok_or_else(|| {
                            format!(
                                "fetch {:?} depends on {id}, which isn't a fetch before it",
                                fetch_id(node)
                            )
                        })
                })
                .collect::<Result<_, _>>()?;
            waits.sort_unstable();
            waits.dedup();
            fetches.push(node);
            waits_for.push(waits.into_boxed_slice());
            Ok(vec![fetches.len() - 1])
        }
    }
}

/// What the fetch in `node` depends on.
fn depends_on<S: PlanState>(node: &PlanNode<S>) -> &[u32] {
    match node {
        PlanNode::Fetch(fetch) => &fetch.depends_on,
        PlanNode::Flatten(flatten) => depends_on(&flatten.node),
        PlanNode::BatchFetch(batch) => &batch.depends_on,
        PlanNode::Subscription(subscription) => &subscription.primary.depends_on,
        _ => &[],
    }
}

fn fetch_id<S: PlanState>(node: &PlanNode<S>) -> Option<u32> {
    match node {
        PlanNode::Fetch(fetch) => Some(fetch.id),
        PlanNode::Flatten(flatten) => fetch_id(&flatten.node),
        PlanNode::BatchFetch(batch) => Some(batch.id),
        PlanNode::Subscription(subscription) => Some(subscription.primary.id),
        _ => None,
    }
}

/// The ids of the fetches in `node`, a batch has all the ones merged into it.
fn fetch_ids<S: PlanState>(node: &PlanNode<S>) -> impl Iterator<Item = u32> + '_ {
    let completes: &[u32] = match node {
        PlanNode::BatchFetch(batch) => &batch.completes,
        _ => &[],
    };
    fetch_id(node).into_iter().chain(completes.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::PlanSchedule;

    #[test]
    fn without_first_drops_the_first_fetch_and_moves_the_rest_up() {
        let schedule = PlanSchedule {
            waits_for: [vec![], vec![0], vec![0, 1], vec![2]]
                .map(Vec::into_boxed_slice)
                .into(),
        };
        let rest = schedule.without_first();
        assert_eq!(rest.len(), 3);
        assert_eq!(rest.waits_for(0), [] as [usize; 0]);
        assert_eq!(rest.waits_for(1), [0]);
        assert_eq!(rest.waits_for(2), [1]);
    }
}
