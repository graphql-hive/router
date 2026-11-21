use ahash::HashSet;
use smallvec::smallvec;
use std::collections::VecDeque;

use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeIndexable},
};

use crate::query_planner::{
    planner::{
        fetch::state::MultiTypeFetchStep,
        plan_nodes::{
            planning::{FetchNode, PlanNode},
            FetchId, FetchIdList,
        },
        query_plan::optimize::{optimize_root_node, optimize_top_level_sequence},
        QueryPlannerOptions,
    },
    state::supergraph_state::{OperationKind, SupergraphState},
    utils::cancellation::CancellationToken,
};

use super::{
    error::QueryPlanError,
    fetch::fetch_graph::FetchGraph,
    plan_nodes::planning::{ParallelNode, QueryPlan, SequenceNode},
};

mod entity_reuse;
mod optimize;

pub static QUERY_PLAN_KIND: &str = "QueryPlan";

#[tracing::instrument(level = "trace", skip_all)]
pub fn build_query_plan_from_fetch_graph(
    fetch_graph: FetchGraph<MultiTypeFetchStep>,
    supergraph: &SupergraphState,
    options: &QueryPlannerOptions,
    cancellation_token: &CancellationToken,
) -> Result<QueryPlan, QueryPlanError> {
    let root_index = fetch_graph.root_index.ok_or(QueryPlanError::NoRoot)?;

    // flat arrays by step index: steps are numbered densely
    let steps = fetch_graph.graph.node_bound();
    // how many of each step's parents aren't planned yet
    let mut unplanned_parents = vec![0u32; steps];
    for step_index in fetch_graph.step_indices() {
        unplanned_parents[step_index.index()] = fetch_graph
            .parents_of(step_index)
            .filter(|edge| edge.source() != root_index)
            .count() as u32;
    }
    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    let mut planned_nodes_count = 0;

    // Initialize the queue with direct children of the root_index.
    // They have no other parents.
    for edge in fetch_graph.children_of(root_index) {
        let child_index = edge.target();
        if unplanned_parents[child_index.index()] == 0 {
            queue.push_back(child_index);
        } else {
            return Err(QueryPlanError::Internal(format!(
                "Root's child ({}) has more than one parent",
                child_index.index()
            )));
        }
    }

    let mut waves: Vec<Vec<PlanNode>> = Vec::new();
    // by step: the fetch ids a child waits for when it depends on that step, its own fetch, or,
    // for a step without a fetch (it never runs), what it waits for itself
    let mut provides: Vec<FetchIdList> = vec![FetchIdList::new(); steps];
    // what finished waves put in the response. An entity fetch may copy entities the fetches it
    // depends on put there.
    let reuse_trees = entity_reuse::StepTrees::new(&fetch_graph, options.entity_reuse);
    let mut entity_reuse = entity_reuse::EntityReuseIndex::new(
        &fetch_graph,
        &reuse_trees,
        supergraph,
        options.entity_reuse,
    );

    while !queue.is_empty() {
        let mut current_wave_nodes: Vec<PlanNode> = Vec::new();
        let mut current_wave_steps = Vec::new();
        let wave_size = queue.len();

        for _ in 0..wave_size {
            let step_index = queue
                .pop_front()
                .ok_or(QueryPlanError::Internal(String::from(
                    "Failed to pop a step from the queue. Queue should not be empty",
                )))?;

            let step_data = fetch_graph.get_step_data(step_index)?;
            // the waves only keep the order, the fetch keeps what it really waits for. The root
            // provides nothing, so it doesn't add anything here
            let mut depends_on: FetchIdList = fetch_graph
                .parents_of(step_index)
                .flat_map(|edge| provides[edge.source().index()].iter().copied())
                .collect();
            depends_on.sort_unstable();
            depends_on.dedup();
            // inline for one or two ids, so the copy doesn't allocate
            let node = PlanNode::from_fetch_step(
                step_data,
                &fetch_graph.arena,
                supergraph,
                options,
                depends_on.clone(),
            );
            provides[step_index.index()] = match node {
                Some(_) => smallvec![step_data.id],
                None => depends_on,
            };
            if let Some(mut node) = node {
                entity_reuse.attach(&mut node, step_index, step_data);
                current_wave_nodes.push(node);
                current_wave_steps.push((step_index, step_data));
            }
            planned_nodes_count += 1;

            cancellation_token.bail_if_cancelled()?;
            for child_edge in fetch_graph.children_of(step_index) {
                let child_index = child_edge.target();
                if child_index == root_index {
                    return Err(QueryPlanError::Internal(String::from(
                        "Visited child step is a root step. It should not happen",
                    )));
                }

                unplanned_parents[child_index.index()] -= 1;
                if unplanned_parents[child_index.index()] == 0 {
                    queue.push_back(child_index);
                }
            }
        }

        for (index, step) in current_wave_steps {
            entity_reuse.add_step(index, step);
        }

        // a wave can end up empty when none of its fetches would ever run
        if !current_wave_nodes.is_empty() {
            waves.push(current_wave_nodes);
        }
    }

    delay_leaf_entity_fetches(&mut waves);

    let mut overall_plan_sequence: Vec<PlanNode> = Vec::with_capacity(waves.len());
    for wave in waves {
        if wave.len() > 1 {
            overall_plan_sequence.push(PlanNode::Parallel(ParallelNode { nodes: wave }));
        } else {
            overall_plan_sequence.extend(wave);
        }
    }

    let total_fetch_nodes = fetch_graph
        .step_indices()
        .filter(|&idx| idx != root_index)
        .count();

    if planned_nodes_count != total_fetch_nodes {
        return Err(QueryPlanError::Internal("Cycle detected".to_string()));
    }

    if overall_plan_sequence.is_empty() {
        if total_fetch_nodes == 0 {
            return Err(QueryPlanError::EmptyPlan);
        }
        // none of the fetches would ever run, so there's nothing to send
        return Ok(QueryPlan::empty());
    }

    // First do light top-level normalization (e.g. flatten nested Sequence wrappers).
    let overall_plan_sequence = optimize_top_level_sequence(overall_plan_sequence);

    let root_node = match overall_plan_sequence.len() == 1 {
        true => overall_plan_sequence.into_iter().next().unwrap(),
        false => PlanNode::Sequence(SequenceNode {
            nodes: overall_plan_sequence,
        }),
    };

    // Then run full recursive optimization + batching rewrites.
    let root_node = optimize_root_node(root_node, supergraph, options)?;

    QueryPlan::new(Some(root_node))
}

/// The entity fetch of a wave entry, `Flatten(Fetch)`, when nothing about it ties it to its wave:
/// no `@include`/`@skip` and no `@fromContext` values.
fn movable_entity_fetch(node: &PlanNode) -> Option<&FetchNode> {
    let PlanNode::Flatten(flatten) = node else {
        return None;
    };
    let fetch = flatten.node.as_fetch()?;
    (fetch.run_if.is_none() && fetch.context.is_none()).then_some(fetch)
}

/// Moves an entity fetch that no other fetch waits for into the nearest later wave that already
/// has an entity fetch to the same subgraph, where batching sends the two in one request.
///
/// Waves are built as early as possible, so `topProducts { inStock }` goes out right after
/// `topProducts`, and `inStock` of products found deeper down goes out three waves later: two
/// requests to the same subgraph. The later wave runs anyway, so moving the first fetch there
/// makes the plan no longer, and nothing waits for its data but the response. It still waits only
/// for what it waited for before, the batch it joins waits for both.
///
/// Queries only: the root fields of a mutation run in order, and those plans are left alone.
fn delay_leaf_entity_fetches(waves: &mut Vec<Vec<PlanNode>>) {
    let mutation = waves.iter().flatten().any(|node| {
        node.as_fetch()
            .is_some_and(|fetch| fetch.operation_kind == Some(OperationKind::Mutation))
    });
    if mutation || waves.len() < 2 {
        return;
    }

    // fetches some other fetch waits for, for its data or to copy entities from it
    let waited_for: HashSet<FetchId> = waves
        .iter()
        .flatten()
        .flat_map(|node| {
            let fetch = match node {
                PlanNode::Flatten(flatten) => flatten.node.as_fetch(),
                other => other.as_fetch(),
            };
            fetch
                .into_iter()
                .flat_map(|fetch| fetch.depends_on.iter().copied())
        })
        .collect();

    // from the back, so a fetch moved into a wave can move on with the ones it joined
    for from in (0..waves.len() - 1).rev() {
        let mut index = 0;
        while index < waves[from].len() {
            let target = movable_entity_fetch(&waves[from][index])
                .filter(|fetch| !waited_for.contains(&fetch.id))
                .and_then(|fetch| {
                    (from + 1..waves.len()).find(|&to| {
                        waves[to].iter().any(|other| {
                            movable_entity_fetch(other)
                                .is_some_and(|other| other.service_name == fetch.service_name)
                        })
                    })
                });
            match target {
                Some(to) => {
                    let node = waves[from].remove(index);
                    waves[to].push(node);
                }
                None => index += 1,
            }
        }
    }
    waves.retain(|wave| !wave.is_empty());
}
