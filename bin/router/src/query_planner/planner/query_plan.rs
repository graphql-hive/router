use std::collections::{HashMap, VecDeque};

use petgraph::{graph::NodeIndex, visit::EdgeRef};

use crate::query_planner::{
    planner::{
        fetch::state::MultiTypeFetchStep,
        plan_nodes::planning::PlanNode,
        query_plan::optimize::{optimize_root_node, optimize_top_level_sequence},
    },
    state::supergraph_state::{OperationKind, SupergraphState},
    utils::cancellation::CancellationToken,
};

use super::{
    error::QueryPlanError,
    fetch::fetch_graph::FetchGraph,
    plan_nodes::planning::{ParallelNode, QueryPlan, ReadySchedule, SequenceNode},
};

mod entity_reuse;
mod optimize;

/// Tracks the in-degree of FetchGraph (DAG) in a dependency graph.
/// The in-degree of a step is the number of its prerequisite parent steps
/// that have not yet been processed. A step is "fulfilled" (ready to be processed)
/// when its in-degree becomes zero.
pub struct InDegree<'a> {
    state: HashMap<NodeIndex, usize>,
    fetch_graph: &'a FetchGraph<MultiTypeFetchStep>,
}

impl<'a> InDegree<'a> {
    pub fn new(fetch_graph: &'a FetchGraph<MultiTypeFetchStep>) -> Result<Self, QueryPlanError> {
        let mut state: HashMap<NodeIndex, usize> = HashMap::new();
        let root_index = fetch_graph.root_index.ok_or(QueryPlanError::NoRoot)?;

        // For all steps, the initial in-degree is set to the total number of their parents
        fetch_graph.bfs(root_index, |step_index, _| {
            state.insert(
                *step_index,
                fetch_graph
                    .parents_of(*step_index)
                    // skip roots
                    .filter(|edge| edge.source() != root_index)
                    .count(),
            );
            false // never stop traversing
        });

        Ok(Self { state, fetch_graph })
    }

    /// Marks a `FetchStep` as processed. This involves decrementing the in-degree
    /// of all its direct children as one of their parent dependencies
    /// has been met.
    pub fn mark_as_processed(&mut self, index: NodeIndex) {
        for edge in self.fetch_graph.children_of(index) {
            let child_index = edge.target();
            let current = self.state.get(&child_index);

            if let Some(in_degree) = current {
                if *in_degree == 0 {
                    panic!("In-degree was 0");
                }

                self.state.insert(child_index, in_degree - 1);
            } else {
                panic!("Attempt to decrease an in-degree of a non-existing step");
            }
        }
    }

    /// Checks if a `FetchStep` is "fulfilled," meaning all its parent dependencies have been met.
    pub fn is_fulfilled(&self, child_index: NodeIndex) -> bool {
        self.state
            .get(&child_index)
            .expect("In-degree record missing")
            == &0
    }
}

pub static QUERY_PLAN_KIND: &str = "QueryPlan";

#[tracing::instrument(level = "trace", skip_all)]
pub fn build_query_plan_from_fetch_graph(
    fetch_graph: FetchGraph<MultiTypeFetchStep>,
    supergraph: &SupergraphState,
    cancellation_token: &CancellationToken,
) -> Result<QueryPlan, QueryPlanError> {
    let root_index = fetch_graph.root_index.ok_or(QueryPlanError::NoRoot)?;

    let mut in_degrees = InDegree::new(&fetch_graph)?;
    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    let mut planned_nodes_count = 0;

    // Initialize the queue with direct children of the root_index.
    // Their in-degrees (calculated in InDegree::new) should be 0.
    for edge in fetch_graph.children_of(root_index) {
        let child_index = edge.target();
        if in_degrees.is_fulfilled(child_index) {
            queue.push_back(child_index);
        } else {
            return Err(QueryPlanError::Internal(format!(
                "Root's child ({}) has more than one parent",
                child_index.index()
            )));
        }
    }

    let mut overall_plan_sequence: Vec<PlanNode> = Vec::new();
    // Steps from finished waves. An entity fetch may copy entities they already put in the response.
    let mut earlier_steps: Vec<NodeIndex> = Vec::new();
    // The steps each fetch copies from, so ready scheduling waits for them too.
    let mut reuse_parents: HashMap<i64, Vec<i64>> = HashMap::new();

    while !queue.is_empty() {
        let mut current_wave_nodes: Vec<PlanNode> = Vec::new();
        let mut current_wave_steps: Vec<NodeIndex> = Vec::new();
        let wave_size = queue.len();

        for _ in 0..wave_size {
            let step_index = queue
                .pop_front()
                .ok_or(QueryPlanError::Internal(String::from(
                    "Failed to pop a step from the queue. Queue should not be empty",
                )))?;

            let step_data = fetch_graph.get_step_data(step_index)?;
            let mut node = PlanNode::from_fetch_step(step_data, supergraph);
            if let Some((reuse, source_steps)) = entity_reuse::find_entity_reuse(
                &fetch_graph,
                step_index,
                &earlier_steps,
                supergraph,
            ) {
                entity_reuse::attach_entity_reuse(&mut node, reuse);
                reuse_parents.insert(step_data.id, source_steps);
            }
            current_wave_nodes.push(node);
            current_wave_steps.push(step_index);
            planned_nodes_count += 1;
            in_degrees.mark_as_processed(step_index);

            for child_edge in fetch_graph.children_of(step_index) {
                cancellation_token.bail_if_cancelled()?;
                let child_index = child_edge.target();
                if child_index == root_index {
                    return Err(QueryPlanError::Internal(String::from(
                        "Visited child step is a root step. It should not happen",
                    )));
                }

                if in_degrees.is_fulfilled(child_index) {
                    queue.push_back(child_index);
                }
            }
        }

        earlier_steps.extend(current_wave_steps);

        if current_wave_nodes.is_empty() {
            return Err(QueryPlanError::Internal(String::from(
                "Wave was empty. It should not happen as the queue was non-empty.",
            )));
        } else if current_wave_nodes.len() == 1 {
            overall_plan_sequence.push(current_wave_nodes.into_iter().next().ok_or(
                QueryPlanError::Internal(String::from("Was was expected to be of length 1")),
            )?);
        } else {
            overall_plan_sequence.push(PlanNode::Parallel(ParallelNode {
                nodes: current_wave_nodes,
            }));
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
        } else {
            return Err(QueryPlanError::Internal(
                "Plan is empty, but graph reported task nodes that were not planned.".to_string(),
            ));
        }
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
    let root_node = optimize_root_node(root_node, supergraph)?;

    Ok(QueryPlan {
        kind: QUERY_PLAN_KIND,
        ready_schedule: build_ready_schedule(&root_node, &fetch_graph, &reuse_parents),
        node: Some(root_node),
    })
}

/// Works out what each fetch of the final plan waits for, from the fetch graph's edges.
/// Gives up (None) on anything that isn't a plain Sequence of waves made of fetches,
/// and on mutations and subscriptions, which have to keep running wave by wave.
fn build_ready_schedule(
    root: &PlanNode,
    fetch_graph: &FetchGraph<MultiTypeFetchStep>,
    reuse_parents: &HashMap<i64, Vec<i64>>,
) -> Option<ReadySchedule> {
    let PlanNode::Sequence(sequence) = root else {
        return None;
    };
    let root_index = fetch_graph.root_index?;

    let mut parents_by_step: HashMap<i64, Vec<i64>> = HashMap::new();
    for index in fetch_graph.step_indices() {
        if index == root_index {
            continue;
        }
        let step = fetch_graph.get_step_data(index).ok()?;
        if !matches!(step.operation_kind, OperationKind::Query) {
            return None;
        }
        let parents = fetch_graph
            .parents_of(index)
            .filter(|edge| edge.source() != root_index)
            .map(|edge| {
                fetch_graph
                    .get_step_data(edge.source())
                    .map(|parent| parent.id)
            })
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let mut parents: Vec<i64> = parents;
        // A fetch that copies entities from earlier fetches has to wait for them as well.
        if let Some(sources) = reuse_parents.get(&step.id) {
            parents.extend(sources);
        }
        parents_by_step.insert(step.id, parents);
    }

    let mut fetches = Vec::new();
    let mut steps_of_fetch = Vec::new();
    for (wave, node) in sequence.nodes.iter().enumerate() {
        match node {
            PlanNode::Parallel(parallel) => {
                for (position, node) in parallel.nodes.iter().enumerate() {
                    fetches.push((wave, position));
                    steps_of_fetch.push(step_ids(node)?);
                }
            }
            node => {
                fetches.push((wave, 0));
                steps_of_fetch.push(step_ids(node)?);
            }
        }
    }

    // Every step has to land in exactly one fetch, or the plan isn't what we think it is.
    let mut fetch_of_step: HashMap<i64, usize> = HashMap::new();
    for (fetch, steps) in steps_of_fetch.iter().enumerate() {
        for step in steps {
            if fetch_of_step.insert(*step, fetch).is_some() {
                return None;
            }
        }
    }
    if fetch_of_step.len() != parents_by_step.len() {
        return None;
    }

    let mut waiting_on = vec![0; fetches.len()];
    let mut dependents = vec![Vec::new(); fetches.len()];
    for (fetch, steps) in steps_of_fetch.iter().enumerate() {
        let mut parents = Vec::new();
        for step in steps {
            for parent in parents_by_step.get(step)? {
                parents.push(*fetch_of_step.get(parent)?);
            }
        }
        parents.sort_unstable();
        parents.dedup();
        parents.retain(|&parent| parent != fetch);
        // Waves come from these same edges, so a parent always sits in an earlier wave.
        if parents
            .iter()
            .any(|&parent| fetches[parent].0 >= fetches[fetch].0)
        {
            return None;
        }
        waiting_on[fetch] = parents.len();
        for parent in parents {
            dependents[parent].push(fetch);
        }
    }

    Some(ReadySchedule {
        fetches,
        waiting_on,
        dependents,
    })
}

/// The fetch step ids a node runs, if it is a single fetch the executor can start on its own.
fn step_ids(node: &PlanNode) -> Option<Vec<i64>> {
    match node {
        PlanNode::Fetch(fetch) => Some(vec![fetch.id]),
        PlanNode::Flatten(flatten) => match flatten.node.as_ref() {
            PlanNode::Fetch(fetch) => Some(vec![fetch.id]),
            _ => None,
        },
        PlanNode::BatchFetch(batch) => Some(batch.fetch_node_ids.clone()),
        PlanNode::Condition(condition) => {
            let mut ids = Vec::new();
            for node in condition
                .if_clause
                .iter()
                .chain(condition.else_clause.iter())
            {
                ids.extend(step_ids(node)?);
            }
            Some(ids)
        }
        _ => None,
    }
}
