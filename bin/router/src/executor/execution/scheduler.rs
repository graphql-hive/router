//! Dependency-aware query plan execution.
//!
//! The query plan is shaped as waves (`Sequence` of `Parallel`) because that's what the
//! plan output and the snapshots look like. Those waves are a presentation detail: the
//! planner records the real fetch-graph dependencies on every fetch node
//! (`depends_on` / `completes`), and this scheduler runs those instead.
//!
//! A wave is a barrier, so a 10ms chain stuck behind a 100ms unrelated fetch waits for
//! it. Here a fetch starts as soon as the fetches it actually needs have been merged
//! into `ctx.data`, so the plan takes as long as its critical path and no longer.

use std::collections::VecDeque;

use ahash::AHashMap;
use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use tracing::warn;

use crate::{
    executor::{
        execution::plan::{condition_node_by_variables, Executor, VariablesMap},
        execution_context::ExecutionContext,
    },
    query_planner::planner::plan_nodes::PlanNode,
};

/// One executable fetch of the plan plus the bookkeeping to schedule it.
struct Job<'exec> {
    node: &'exec PlanNode,
    /// Fetch ids that are done once this job's result is merged. More than one for a
    /// `BatchFetch`, which stands in for every fetch it merged.
    completes: &'exec [i64],
    /// Fetch ids this job waits for.
    depends_on: &'exec [i64],
    /// Dependencies that have not completed yet.
    remaining: usize,
    /// False when a `@skip`/`@include` above this fetch does not hold. Such a job is
    /// never sent, it only releases whatever waits on it - otherwise its dependents
    /// would block forever.
    active: bool,
}

impl<'exec> Executor<'exec> {
    /// Runs the plan by its fetch-graph dependencies.
    ///
    /// Returns `false` without touching `ctx` when the plan has a shape this scheduler
    /// does not handle (`@defer`, a subscription node) - the caller falls back to wave
    /// execution.
    pub async fn execute_plan_dependency_aware(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        root: &'exec PlanNode,
    ) -> bool {
        let mut jobs = Vec::new();
        if !collect_jobs(root, true, self.variable_values, &mut jobs) {
            return false;
        }

        // id -> the job that completes it, and the reverse edges we walk on completion.
        let mut job_by_id: AHashMap<i64, usize> = AHashMap::with_capacity(jobs.len());
        for (index, job) in jobs.iter().enumerate() {
            for id in job.completes {
                job_by_id.insert(*id, index);
            }
        }

        let mut dependents: AHashMap<i64, Vec<usize>> = AHashMap::new();
        for (index, job) in jobs.iter_mut().enumerate() {
            let mut remaining = 0;
            for id in job.depends_on {
                match job_by_id.get(id) {
                    // A fetch cannot wait for itself. The planner already drops these,
                    // this is here so a bad plan cannot deadlock the request.
                    Some(&producer) if producer == index => continue,
                    Some(_) => {
                        remaining += 1;
                        dependents.entry(*id).or_default().push(index);
                    }
                    // Nothing in this plan produces that id. Waiting for it would hang.
                    None => warn!(
                        fetch_id = id,
                        "query plan dependency has no producing fetch, ignoring it"
                    ),
                }
            }
            job.remaining = remaining;
        }

        // FIFO, so fetches start in plan order and request logs stay comparable to the
        // wave executor's.
        let mut ready: VecDeque<usize> = jobs
            .iter()
            .enumerate()
            .filter(|(_, job)| job.remaining == 0)
            .map(|(index, _)| index)
            .collect();
        let mut running = FuturesUnordered::new();
        let mut unfinished = jobs.len();

        while unfinished > 0 {
            while let Some(index) = ready.pop_front() {
                // Preparing the request needs `ctx.data`, running it does not - which is
                // what lets us merge a result and start its dependents in the same loop.
                let future = match jobs[index].active {
                    true => self.prepare_job_future(jobs[index].node, &ctx.data),
                    false => None,
                };

                match future {
                    Some(future) => running.push(future.map(move |result| (index, result))),
                    // Nothing to send: a skipped condition, or an entity fetch with no
                    // representations. Still counts as done for everything behind it.
                    None => {
                        unfinished -= 1;
                        release(&mut jobs, &dependents, index, &mut ready);
                    }
                }
            }

            let Some((index, result)) = running.next().await else {
                // Nothing ready and nothing in flight, yet jobs remain: a cycle, or a
                // dependency nobody completes. Stop instead of hanging the request.
                warn!(
                    unfinished,
                    "dependency-aware execution stalled, {unfinished} fetches were skipped"
                );
                break;
            };

            // Order matters: the result has to be in `ctx.data` before the fetches that
            // need it are prepared, because that's where their representations come from.
            self.process_job_result(ctx, result);
            unfinished -= 1;
            release(&mut jobs, &dependents, index, &mut ready);
        }

        true
    }
}

fn release<'exec>(
    jobs: &mut [Job<'exec>],
    dependents: &AHashMap<i64, Vec<usize>>,
    index: usize,
    ready: &mut VecDeque<usize>,
) {
    let completes: &'exec [i64] = jobs[index].completes;

    for id in completes {
        let Some(waiting) = dependents.get(id) else {
            continue;
        };

        for &dependent in waiting {
            let job = &mut jobs[dependent];
            if job.remaining > 0 {
                job.remaining -= 1;
                if job.remaining == 0 {
                    ready.push_back(dependent);
                }
            }
        }
    }
}

/// Flattens the wave-shaped plan into the fetches it contains.
///
/// Returns `false` when it hits a node this scheduler does not run.
fn collect_jobs<'exec>(
    node: &'exec PlanNode,
    active: bool,
    variable_values: &Option<VariablesMap>,
    out: &mut Vec<Job<'exec>>,
) -> bool {
    let mut push = |completes: &'exec [i64], depends_on: &'exec [i64]| {
        out.push(Job {
            node,
            completes,
            depends_on,
            remaining: 0,
            active,
        });
        true
    };

    match node {
        PlanNode::Fetch(fetch) => push(std::slice::from_ref(&fetch.id), &fetch.depends_on),
        PlanNode::BatchFetch(batch) => push(&batch.completes, &batch.depends_on),
        PlanNode::Flatten(flatten) => match flatten.node.as_ref() {
            PlanNode::Fetch(fetch) => push(std::slice::from_ref(&fetch.id), &fetch.depends_on),
            _ => false,
        },
        PlanNode::Sequence(sequence) => sequence
            .nodes
            .iter()
            .all(|child| collect_jobs(child, active, variable_values, out)),
        PlanNode::Parallel(parallel) => parallel
            .nodes
            .iter()
            .all(|child| collect_jobs(child, active, variable_values, out)),
        PlanNode::Condition(condition) => {
            // Both clauses are collected. The one that is not taken is marked inactive so
            // it still completes and releases its dependents, without being sent.
            let taken = condition_node_by_variables(condition, variable_values);

            [
                condition.if_clause.as_deref(),
                condition.else_clause.as_deref(),
            ]
            .into_iter()
            .flatten()
            .all(|clause| {
                let clause_active =
                    active && taken.is_some_and(|taken| std::ptr::eq(taken, clause));
                collect_jobs(clause, clause_active, variable_values, out)
            })
        }
        // Subscriptions are peeled off before the plan gets here, and `@defer` needs the
        // wave executor's incremental delivery. Fall back for both.
        PlanNode::Subscription(_) | PlanNode::Defer(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::{
        ast::operation::SubgraphFetchOperation,
        planner::plan_nodes::{
            ConditionNode, DeferNode, DeferPrimary, FetchNode, FlattenNode, ParallelNode,
            SequenceNode,
        },
    };

    fn fetch(id: i64, depends_on: &[i64]) -> PlanNode {
        PlanNode::Fetch(Box::new(FetchNode {
            id,
            depends_on: depends_on.into(),
            service_name: "a".to_string(),
            variable_usages: None,
            operation_kind: None,
            operation: SubgraphFetchOperation {
                document_str: "{ __typename }".into(),
                hash: 0,
                name_write_position: 0,
            },
            custom_scalar_paths: None,
            requires: None,
            planner_requires: (),
            input_rewrites: None,
            output_rewrites: None,
        }))
    }

    fn collect(node: &PlanNode, variables: &Option<VariablesMap>) -> Option<Vec<(Vec<i64>, bool)>> {
        let mut jobs = Vec::new();
        collect_jobs(node, true, variables, &mut jobs).then(|| {
            jobs.iter()
                .map(|job| (job.completes.to_vec(), job.active))
                .collect()
        })
    }

    #[test]
    fn collects_every_fetch_of_a_wave_shaped_plan() {
        let plan = PlanNode::Sequence(SequenceNode {
            nodes: vec![
                PlanNode::Parallel(ParallelNode {
                    nodes: vec![fetch(1, &[]), fetch(2, &[])],
                }),
                PlanNode::Parallel(ParallelNode {
                    nodes: vec![
                        PlanNode::Flatten(FlattenNode {
                            path: Default::default(),
                            node: Box::new(fetch(3, &[1])),
                        }),
                        fetch(4, &[2]),
                    ],
                }),
            ],
        });

        assert_eq!(
            collect(&plan, &None),
            Some(vec![
                (vec![1], true),
                (vec![2], true),
                (vec![3], true),
                (vec![4], true)
            ])
        );
    }

    /// A fetch under a condition that does not hold is still collected, only inactive.
    /// If it were dropped, anything waiting on it would never be scheduled.
    #[test]
    fn keeps_skipped_condition_branches_as_inactive_jobs() {
        let plan = PlanNode::Condition(ConditionNode {
            condition: "withReviews".to_string(),
            if_clause: Some(Box::new(fetch(1, &[]))),
            else_clause: Some(Box::new(fetch(2, &[]))),
        });

        let variables: Option<VariablesMap> = Some(VariablesMap::from([(
            "withReviews".to_string(),
            true.into(),
        )]));
        assert_eq!(
            collect(&plan, &variables),
            Some(vec![(vec![1], true), (vec![2], false)])
        );

        // No value for the variable: neither branch runs, both still complete.
        assert_eq!(
            collect(&plan, &None),
            Some(vec![(vec![1], false), (vec![2], false)])
        );
    }

    #[test]
    fn bails_out_on_defer_so_the_caller_falls_back_to_waves() {
        let plan = PlanNode::Defer(Box::new(DeferNode {
            primary: DeferPrimary {
                subselection: None,
                node: Some(Box::new(fetch(1, &[]))),
            },
            deferred: vec![],
        }));

        assert_eq!(collect(&plan, &None), None);
    }
}
