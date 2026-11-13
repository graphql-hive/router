//! Which fetches of a plan wait for which, what the executor schedules on.
//!
//! The planner tells each fetch which fetches it reads from (`depends_on`, by [`FetchId`]).
//! When the plan is made, that's turned into a [`PlanSchedule`] by position, so the executor starts
//! a fetch the moment those are done, and executable nodes don't keep any ids for it.

use smallvec::SmallVec;

use super::{FetchId, PlanNode, PlanState, Planning};
use crate::query_planner::planner::error::QueryPlanError;

/// The fetches of a plan by position, the order [`plan_fetches`] lists them in, and which ones
/// wait for which. Kept as flat arrays: a plan is made once and run many times.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanSchedule {
    /// How many fetches each fetch waits for.
    waits: Box<[u32]>,
    /// Where each fetch's dependents start in `dependents`, with one more at the end.
    dependents_start: Box<[u32]>,
    /// The fetches that wait for each fetch, back to back.
    dependents: Box<[u32]>,
}

impl PlanSchedule {
    /// The schedule of `node`. Fails when a fetch depends on one that doesn't come before it in
    /// the plan, so a schedule never has a cycle.
    pub fn new(node: Option<&PlanNode<Planning>>) -> Result<Self, QueryPlanError> {
        let fetches = node.map(plan_fetches).unwrap_or_default();
        let position = positions_by_id(&fetches)?;
        let mut dependencies: Vec<(u32, u32)> = Vec::new();
        for (index, fetch) in fetches.iter().enumerate() {
            let Some((id, depends_on)) = id_and_depends_on(fetch) else {
                continue;
            };
            let index = index as u32;
            for &dependency in depends_on {
                let dependency_index = position
                    .get(dependency.index())
                    .copied()
                    .filter(|&dependency_index| dependency_index < index)
                    .ok_or(QueryPlanError::DependsOnLaterFetch {
                        fetch: id,
                        dependency,
                    })?;
                dependencies.push((dependency_index, index));
            }
        }
        Ok(Self::from_dependencies(fetches.len(), dependencies))
    }

    /// The schedule of `fetch_count` fetches, from `(dependency, dependent)` pairs by position: the
    /// dependent waits for the dependency.
    pub(crate) fn from_dependencies(fetch_count: usize, mut dependencies: Vec<(u32, u32)>) -> Self {
        dependencies.sort_unstable();
        // two ids merged into one batch are one fetch
        dependencies.dedup();
        let mut waits = vec![0; fetch_count];
        let mut dependents_start = vec![0; fetch_count + 1];
        for &(dependency, dependent) in &dependencies {
            waits[dependent as usize] += 1;
            dependents_start[dependency as usize + 1] += 1;
        }
        for index in 0..fetch_count {
            dependents_start[index + 1] += dependents_start[index];
        }
        Self {
            waits: waits.into(),
            dependents_start: dependents_start.into(),
            dependents: dependencies
                .into_iter()
                .map(|(_, dependent)| dependent)
                .collect(),
        }
    }

    /// How many fetches the plan has.
    pub fn fetch_count(&self) -> usize {
        self.waits.len()
    }

    /// How many fetches each fetch waits for.
    pub fn waits(&self) -> &[u32] {
        &self.waits
    }

    /// The fetches that wait for the `index`th one.
    pub fn dependents(&self, index: usize) -> &[u32] {
        let start = self.dependents_start[index] as usize;
        let end = self.dependents_start[index + 1] as usize;
        &self.dependents[start..end]
    }

    /// The schedule without the first fetch, for the rest of a subscription's plan that runs on
    /// every event: the subscription fetch is done by then.
    pub(super) fn after_first_fetch(&self) -> Self {
        let dependencies = (1..self.fetch_count())
            .flat_map(|dependency| {
                self.dependents(dependency)
                    .iter()
                    .map(move |dependent| (dependency as u32 - 1, dependent - 1))
            })
            .collect();
        Self::from_dependencies(self.fetch_count().saturating_sub(1), dependencies)
    }
}

/// The fetches of `node` in the plan's order: every node that isn't a `Sequence` or a `Parallel`.
pub fn plan_fetches<S: PlanState>(node: &PlanNode<S>) -> SmallVec<[&PlanNode<S>; 16]> {
    let mut fetches = SmallVec::new();
    push_fetches(node, &mut fetches);
    fetches
}

fn push_fetches<'p, S: PlanState>(
    node: &'p PlanNode<S>,
    fetches: &mut SmallVec<[&'p PlanNode<S>; 16]>,
) {
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

/// Each fetch's position, by id. Fetch ids count up from one, so a flat array holds them.
/// `u32::MAX` where no fetch has that id. Fails when two fetches have one id, a dependency on it
/// would pick one of them.
fn positions_by_id(fetches: &[&PlanNode<Planning>]) -> Result<Vec<u32>, QueryPlanError> {
    let ids = |fetch| {
        id_and_depends_on(fetch)
            .map(|(id, _)| id)
            .into_iter()
            .chain(completes(fetch))
    };
    // one more than the highest id
    let id_bound = fetches
        .iter()
        .flat_map(|fetch| ids(fetch))
        .map(|id| id.index() + 1)
        .max()
        .unwrap_or(0);
    let mut position = vec![u32::MAX; id_bound];
    for (index, fetch) in fetches.iter().enumerate() {
        let index = index as u32;
        for id in ids(fetch) {
            // a batch has its own id in `completes` too
            match position[id.index()] {
                u32::MAX => position[id.index()] = index,
                other if other == index => {}
                _ => return Err(QueryPlanError::DuplicateFetchId { id }),
            }
        }
    }
    Ok(position)
}

/// A fetch's id and the fetches it depends on, `None` for a node that isn't a fetch.
fn id_and_depends_on(node: &PlanNode<Planning>) -> Option<(FetchId, &[FetchId])> {
    match node {
        PlanNode::Fetch(fetch) => Some((fetch.id, &fetch.depends_on)),
        PlanNode::Flatten(flatten) => id_and_depends_on(&flatten.node),
        PlanNode::BatchFetch(batch) => Some((batch.id, &batch.depends_on)),
        PlanNode::Subscription(subscription) => {
            Some((subscription.primary.id, &subscription.primary.depends_on))
        }
        _ => None,
    }
}

fn completes(node: &PlanNode<Planning>) -> impl Iterator<Item = FetchId> + '_ {
    let completes: &[FetchId] = match node {
        PlanNode::BatchFetch(batch) => &batch.completes,
        _ => &[],
    };
    completes.iter().copied()
}

#[cfg(test)]
mod tests {
    use super::PlanSchedule;

    #[test]
    fn after_first_fetch_drops_it_and_moves_the_rest_up() {
        // 0 → 1, 0 → 2, 1 → 2, 2 → 3
        let schedule = PlanSchedule::from_dependencies(4, vec![(0, 1), (0, 2), (1, 2), (2, 3)]);
        assert_eq!(schedule.waits(), [0, 1, 2, 1]);
        assert_eq!(schedule.dependents(0), [1, 2]);

        let rest = schedule.after_first_fetch();
        assert_eq!(rest.waits(), [0, 1, 1]);
        assert_eq!(rest.dependents(0), [1]);
        assert_eq!(rest.dependents(1), [2]);
        assert_eq!(rest.dependents(2), [] as [u32; 0]);
    }
}
