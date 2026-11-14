use smallvec::smallvec;
use std::collections::VecDeque;

use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, NodeIndexable},
};

use crate::query_planner::{
    planner::{
        fetch::state::MultiTypeFetchStep,
        plan_nodes::{planning::PlanNode, FetchIdList},
        query_plan::optimize::{optimize_root_node, optimize_top_level_sequence},
        QueryPlannerOptions,
    },
    state::supergraph_state::SupergraphState,
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

    let mut overall_plan_sequence: Vec<PlanNode> = Vec::new();
    // by step: the fetch ids a child waits for when it depends on that step, its own fetch, or,
    // for a step without a fetch (it never runs), what it waits for itself
    let mut provides: Vec<FetchIdList> = vec![FetchIdList::new(); steps];
    // what finished waves put in the response. An entity fetch may copy entities the fetches it
    // depends on put there.
    let mut entity_reuse = entity_reuse::EntityReuseIndex::new(
        &fetch_graph,
        supergraph,
        options.experimental_entity_reuse,
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
            let node =
                PlanNode::from_fetch_step(step_data, supergraph, options, depends_on.clone());
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
        if current_wave_nodes.len() > 1 {
            overall_plan_sequence.push(PlanNode::Parallel(ParallelNode {
                nodes: current_wave_nodes,
            }));
        } else {
            overall_plan_sequence.extend(current_wave_nodes);
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
