use std::collections::{HashMap, HashSet};

use petgraph::{graph::NodeIndex, visit::EdgeRef};

use super::{
    error::QueryPlanError,
    fetch::fetch_graph::FetchGraph,
    plan_nodes::{QueryPlan, QueryPlanNode},
};

/// Tracks the in-degree of FetchGraph (DAG) in a dependency graph.
/// The in-degree of a step is the number of its prerequisite parent steps
/// that have not yet been processed. A step is "fulfilled" (ready to be processed)
/// when its in-degree becomes zero.
pub struct InDegree {
    state: HashMap<NodeIndex, usize>,
    fetch_graph: FetchGraph,
}

impl InDegree {
    pub fn new(fetch_graph: FetchGraph) -> Result<Self, QueryPlanError> {
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
                if in_degree == &0 {
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

/// The entire process of planning the FetchSteps is a topological sort.
/// The goal is to process steps only after their prerequisite parent steps are completed.
/// I modified Kahn's algorithm a bit and used the InDegree class to keep track of which Nodes (FetchSteps)
/// are ready to be placed in the QueryPlan.
///
/// We're basically transforming a Directed Acyclic Graph into a Tree.
///
/// Initially, I used a different approach when I also tried to find the end of each Sequence,
/// but I did hit an issue with a FetchStep depending on more than one parent.
pub fn build_query_plan_from_fetch_graph(
    fetch_graph: FetchGraph,
) -> Result<QueryPlan, QueryPlanError> {
    let initial_parallel_steps: Vec<NodeIndex> = fetch_graph
        .children_of(fetch_graph.root_index.ok_or(QueryPlanError::NoRoot)?)
        .map(|edge| edge.target())
        .collect();

    let mut in_degree = InDegree::new(fetch_graph)?;
    let overall_plan_result =
        orchestrate_layer_processing(&mut in_degree, initial_parallel_steps, vec![])?;

    if !overall_plan_result.pending_steps.is_empty() {
        return Err(QueryPlanError::UnexpectedPendingState);
    }

    if overall_plan_result.nodes.is_empty() {
        return Err(QueryPlanError::EmptyPlan);
    }

    Ok(QueryPlan {
        root: flatten_sequence(overall_plan_result.nodes),
    })
}

struct FetchResult {
    /// Final Node for a given step
    node: QueryPlanNode,
    pending_steps: Vec<NodeIndex>,
}

/// Plans an individual `FetchStepData`
fn plan_fetch_step(
    in_degree: &mut InDegree,
    step_index: NodeIndex,
) -> Result<FetchResult, QueryPlanError> {
    let mut ready_steps: Vec<NodeIndex> = vec![];
    let mut pending_steps: Vec<NodeIndex> = vec![];

    in_degree.mark_as_processed(step_index);

    for child_edge in in_degree.fetch_graph.children_of(step_index) {
        // If the child belongs to more than one parent
        // we need to defer its process.
        //    S0
        // P1   P2   | P - Parallel
        //  \   /    | S - Step
        //   \ /
        //    S  (S.parents.length == 2)
        // We can't add S as a child of P1 or P2, it needs to be a child of all parents.
        let child_index = child_edge.target();
        if in_degree.is_fulfilled(child_index)
            && in_degree.fetch_graph.parents_of(child_index).count() == 1
        {
            ready_steps.push(child_index);
        } else {
            pending_steps.push(child_index);
        }
    }

    // it's a leaf node in the sequence
    if ready_steps.is_empty() {
        return Ok(FetchResult {
            node: in_degree.fetch_graph.get_step_data(step_index)?.into(),
            pending_steps,
        });
    }

    let result = orchestrate_layer_processing(in_degree, ready_steps, pending_steps)?;

    let mut nodes: Vec<QueryPlanNode> =
        vec![in_degree.fetch_graph.get_step_data(step_index)?.into()];
    nodes.extend(result.nodes);

    Ok(FetchResult {
        node: flatten_sequence(nodes),
        pending_steps: result.pending_steps,
    })
}

struct ParallelResult {
    /// Node representation of the layer
    node: QueryPlanNode,
    /// Steps that become ready after the layer was processed.
    ready_steps: Vec<NodeIndex>,
    /// Steps that remain pending after the layer was processed.
    pending_steps: Vec<NodeIndex>,
}

/// Plans a single layer of concurrently executable `FetchStepData`s.
///
/// Accepts
/// - `concurrent_ready_steps` - Steps that are all ready to be processed in parallel
/// - `previously_pending_steps` - Steps that were pending from previous processing stages
fn plan_parallel_step_layer(
    in_degree: &mut InDegree,
    concurrent_ready_steps: Vec<NodeIndex>,
    previously_pending_steps: Vec<NodeIndex>,
) -> Result<ParallelResult, QueryPlanError> {
    let mut new_pending_steps: HashSet<NodeIndex> = HashSet::new();
    for pending in previously_pending_steps {
        new_pending_steps.insert(pending);
    }

    let mut nodes: Vec<QueryPlanNode> = Vec::with_capacity(concurrent_ready_steps.len());
    for step_index in concurrent_ready_steps {
        let fetch_result = plan_fetch_step(in_degree, step_index)?;
        nodes.push(fetch_result.node);
        for pending in fetch_result.pending_steps {
            new_pending_steps.insert(pending);
        }
    }

    let mut next_ready_steps: Vec<NodeIndex> = vec![];
    let mut remaining_pending_steps: Vec<NodeIndex> = vec![];
    for step in new_pending_steps.iter() {
        if in_degree.is_fulfilled(*step) {
            next_ready_steps.push(*step);
        } else {
            remaining_pending_steps.push(*step);
        }
    }

    Ok(ParallelResult {
        node: flatten_parallel(nodes),
        ready_steps: next_ready_steps,
        pending_steps: remaining_pending_steps,
    })
}

struct SequenceResult {
    /// A list of `QueryPlanNode`s, each representing a processed layer, forming a sequence.
    nodes: Vec<QueryPlanNode>,
    pending_steps: Vec<NodeIndex>,
}

/// Orchestrates the processing of FetchSteps layer by layer, building a sequence of plan nodes.
/// It repeatedly calls `plan_parallel_step_layer` for batches of ready steps until no more
/// steps can be processed.
///
/// - `initial_ready_steps` - The initial set of FetchSteps that are ready to be processed.
/// - `initial_pending_steps` - Any FetchSteps that are already pending from a higher context.
fn orchestrate_layer_processing(
    in_degree: &mut InDegree,
    initial_ready_steps: Vec<NodeIndex>,
    initial_pending_steps: Vec<NodeIndex>,
) -> Result<SequenceResult, QueryPlanError> {
    if initial_ready_steps.is_empty() {
        return Ok(SequenceResult {
            nodes: vec![],
            pending_steps: initial_pending_steps,
        });
    }

    let parallel_result =
        plan_parallel_step_layer(in_degree, initial_ready_steps, initial_pending_steps)?;
    let next_layer_result = orchestrate_layer_processing(
        in_degree,
        parallel_result.ready_steps,
        parallel_result.pending_steps,
    )?;

    let mut nodes = vec![parallel_result.node];
    nodes.extend(next_layer_result.nodes);

    Ok(SequenceResult {
        nodes,
        pending_steps: next_layer_result.pending_steps,
    })
}

fn flatten_sequence(nodes: Vec<QueryPlanNode>) -> QueryPlanNode {
    let flattened: Vec<QueryPlanNode> = nodes
        .into_iter()
        .flat_map(|node| match node {
            QueryPlanNode::Sequence(nested) => flatten_sequence(nested).into_nodes(),
            other => vec![other],
        })
        .collect();

    match flattened.len() {
        0 => QueryPlanNode::Sequence(vec![]),
        1 => flattened.into_iter().next().unwrap(),
        _ => QueryPlanNode::Sequence(flattened),
    }
}

fn flatten_parallel(nodes: Vec<QueryPlanNode>) -> QueryPlanNode {
    let flattened: Vec<QueryPlanNode> = nodes
        .into_iter()
        .flat_map(|node| match node {
            QueryPlanNode::Parallel(nested) => flatten_parallel(nested).into_nodes(),
            other => vec![other],
        })
        .collect();

    match flattened.len() {
        0 => QueryPlanNode::Parallel(vec![]),
        1 => flattened.into_iter().next().unwrap(),
        _ => QueryPlanNode::Parallel(flattened),
    }
}
