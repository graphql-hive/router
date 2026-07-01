//! This module solves the problem of finding the "best" execution plan for a
//! query that can be satisfied in many different ways by various subgraphs.
//!
//! The "best" plan is defined by a cost model that penalizes crossing
//! subgraph boundaries, and favors plans that are "local" to a single subgraph.
//!
//! A naive search to find the best combination would be exponentially slow (we were there...),
//! as the number of possible plans is the product of the number of choices for each part of
//! the query. This module implements a greedy algorithm to make this problem tractable.
//!
//! The key components are:
//!
//! 1.  The query is broken down into independent decision points.
//!     For each point, there is a set of `Alternatives`, and each alternative is a
//!     `Candidate` plan fragment. The goal is to pick exactly one `Candidate` from
//!     each set of `Alternatives`.
//!
//! 2.  Singletons (fields with only one candidate) are merged eagerly before the main
//!     search. This reduces the number of candidates to consider.
//!
//! 3.  The main greedy search iterates over the remaining decision points and at each
//!     step picks the candidate that adds the smallest cost to the current plan.
//!     Cost computation uses `calculate_added_cost_of_merge` to avoid expensive
//!     temporary tree clones.
//!
//! 4.  Sets of `Alternatives` are sorted by size (fewest choices first) to reduce
//!     the branching factor and find a complete plan sooner.
//!
//! 5.  The `Candidate` struct uses `LazyTransform` to ensure that the
//!     potentially expensive work of building a `QueryTree` is only done
//!     if a candidate is actually evaluated, and never more than once.

use fixedbitset::FixedBitSet;
use lazy_init::LazyTransform;
use priority_queue::PriorityQueue;
use rustc_hash::FxHashMap;
use std::{cmp::Reverse, rc::Rc};

use crate::{
    graph::{edge::Edge, error::GraphError, Graph},
    planner::{
        error::QueryPlanError,
        tree::{
            query_tree::QueryTree,
            query_tree_node::{MutationFieldPosition, QueryTreeNode},
        },
        walker::{path::OperationPath, ResolvedOperation},
    },
    state::supergraph_state::OperationKind,
    utils::cancellation::CancellationToken,
};

type PathAndPosition<'graph> = (OperationPath<'graph>, MutationFieldPosition);
type QueryTreeResult = Result<QueryTree, GraphError>;
type LazyQueryTree<'graph> = LazyTransform<PathAndPosition<'graph>, QueryTreeResult>;

/// The high-penalty cost for crossing a subgraph boundary or satisfying a requirement.
/// This is the primary driver of the optimization, encouraging plans with fewer subgraphs.
const CROSS_SUBGRAPH_COST: u64 = 1000;
const FIELD_COST: u64 = 1;
const EXACT_COMBINATION_LIMIT: usize = 65_536;

type SteinerNodeId = usize;
type SteinerEdgeId = usize;

/// A lazily-evaluated, potential piece of the final query plan.
/// It represents one of many possible ways to resolve a part of the query.
#[derive(Clone)]
struct Candidate<'graph> {
    /// The actual `QueryTree` for this candiate, computed only when first needed.
    tree: LazyQueryTree<'graph>,
}

/// A group of alternatives - all possible ways to resolve one field.
/// The final plan must pick exactly one Candidate from each group.
type Alternatives<'graph> = Vec<Candidate<'graph>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum NodeContext {
    Child,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NodeFingerprint {
    node_index: usize,
    edge_from_parent: Option<usize>,
    selection_alias: Option<String>,
    selection_arguments: Option<String>,
    condition: Option<crate::ast::merge_path::Condition>,
    mutation_field_position: MutationFieldPosition,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SteinerNodeKey {
    context: NodeContext,
    path: Vec<NodeFingerprint>,
}

#[derive(Clone)]
struct SteinerEdge {
    from: SteinerNodeId,
    to: SteinerNodeId,
    weight: u64,
    candidate: Option<(usize, usize)>,
}

struct SteinerGraph {
    edges: Vec<SteinerEdge>,
    incoming: Vec<Vec<SteinerEdgeId>>,
    outgoing: Vec<Vec<SteinerEdgeId>>,
    edge_by_endpoints: FxHashMap<(SteinerNodeId, SteinerNodeId), SteinerEdgeId>,
}

struct SteinerTree {
    nodes: Vec<bool>,
    edges: Vec<bool>,
    terminals: Vec<SteinerNodeId>,
}

struct FlowState {
    saturated_edges: Vec<bool>,
    marked_edges: Vec<bool>,
    node_feeding_terminals: Vec<FixedBitSet>,
}

struct SteinerBuildResult {
    graph: SteinerGraph,
    tree: SteinerTree,
    candidates: Vec<Vec<QueryTree>>,
}

impl NodeFingerprint {
    fn from_node(node: &QueryTreeNode) -> Self {
        Self {
            node_index: node.node_index.index(),
            edge_from_parent: node.edge_from_parent.map(|edge| edge.index()),
            selection_alias: node.selection_alias().map(str::to_owned),
            selection_arguments: node.selection_arguments().map(|args| format!("{args:?}")),
            condition: node.condition.clone(),
            mutation_field_position: node.mutation_field_position,
        }
    }
}

impl SteinerGraph {
    fn new() -> Self {
        Self {
            edges: Vec::new(),
            incoming: vec![Vec::new()],
            outgoing: vec![Vec::new()],
            edge_by_endpoints: FxHashMap::default(),
        }
    }

    fn add_node(&mut self) -> SteinerNodeId {
        let node_id = self.incoming.len();
        self.incoming.push(Vec::new());
        self.outgoing.push(Vec::new());
        node_id
    }

    fn add_edge(
        &mut self,
        from: SteinerNodeId,
        to: SteinerNodeId,
        weight: u64,
        candidate: Option<(usize, usize)>,
    ) -> SteinerEdgeId {
        if candidate.is_none() {
            if let Some(edge_id) = self.edge_by_endpoints.get(&(from, to)).copied() {
                let edge = &mut self.edges[edge_id];
                if weight < edge.weight {
                    edge.weight = weight;
                }
                return edge_id;
            }
        }

        let edge_id = self.edges.len();
        self.edges.push(SteinerEdge {
            from,
            to,
            weight,
            candidate,
        });
        self.incoming[to].push(edge_id);
        self.outgoing[from].push(edge_id);
        if candidate.is_none() {
            self.edge_by_endpoints.insert((from, to), edge_id);
        }
        edge_id
    }
}

impl<'graph> Candidate<'graph> {
    fn new(path: OperationPath<'graph>, mutation_pos: MutationFieldPosition) -> Self {
        Self {
            tree: LazyTransform::new((path, mutation_pos)),
        }
    }

    #[inline]
    fn get_tree(&self, graph: &Graph) -> Result<QueryTree, QueryPlanError> {
        Ok(self
            .tree
            .get_or_create(|(p, mp)| QueryTree::from_path(graph, &p, mp))
            .clone()?)
    }
}

/// Build the list of alternatives for each field in the query.
/// Returns a list where each entry is all the ways to resolve one field.
fn prepare_alternatives<'graph>(operation: ResolvedOperation<'graph>) -> Vec<Alternatives<'graph>> {
    let is_mutation = matches!(operation.operation_kind, OperationKind::Mutation);
    let mut per_leaf_alternatives: Vec<Alternatives<'graph>> = Vec::new();

    for (index, root_field_options) in operation.root_field_groups.into_iter().enumerate() {
        let mutation_field_position: MutationFieldPosition = is_mutation.then_some(index);

        let leaf_alternatives: Vec<Alternatives<'graph>> = root_field_options
            .into_iter()
            .map(|paths_to_leaf| {
                paths_to_leaf
                    .into_iter()
                    .map(|op| Candidate::new(op, mutation_field_position))
                    .collect::<Alternatives>()
            })
            .collect();

        per_leaf_alternatives.extend(leaf_alternatives);
    }

    // Sort alternatives by length in ascending order.
    per_leaf_alternatives.sort_by_key(|alternatives| alternatives.len());

    per_leaf_alternatives
}

/// Merge fields that have only one possible resolution.
///
/// Returns:
/// - The merged base tree, if any singletons were found.
/// - The remaining alternatives, which still have more than one candidate.
fn merge_singleton_alternatives<'graph>(
    graph: &Graph,
    per_leaf_alternatives: Vec<Alternatives<'graph>>,
) -> Result<(Option<QueryTree>, Vec<Alternatives<'graph>>), QueryPlanError> {
    // Split into singletons (1 candidate) and non-singletons (2+ candidates).
    let (singletons, remaining): (Vec<_>, Vec<_>) = per_leaf_alternatives
        .into_iter()
        .partition(|a| a.len() == 1);

    // Merge all singletons into one base tree.
    let mut base_tree: Option<QueryTree> = None;
    for alternatives in singletons {
        let candidate = alternatives
            .into_iter()
            .next()
            .expect("singleton has one candidate");
        let candidate_tree = candidate.get_tree(graph)?;
        match base_tree.as_mut() {
            // Merge into the existing base tree.
            Some(tree) => Rc::make_mut(&mut tree.root).merge_nodes(&candidate_tree.root),
            // This is the first singleton - it becomes the base tree.
            None => base_tree = Some(candidate_tree),
        }
    }

    Ok((base_tree, remaining))
}

fn build_steiner_graph(
    graph: &Graph,
    base_tree: Option<&QueryTree>,
    per_leaf_alternatives: &[Alternatives],
) -> Result<SteinerBuildResult, QueryPlanError> {
    let mut steiner_graph = SteinerGraph::new();
    let mut node_ids = FxHashMap::<SteinerNodeKey, SteinerNodeId>::default();
    let mut tree = SteinerTree {
        nodes: vec![true],
        edges: Vec::new(),
        terminals: Vec::with_capacity(per_leaf_alternatives.len()),
    };

    if let Some(base_tree) = base_tree {
        let mut path = Vec::new();
        ingest_tree_nodes(
            graph,
            &mut steiner_graph,
            &mut node_ids,
            0,
            &base_tree.root.children,
            &mut path,
            Some(&mut tree),
        );
    }

    let mut candidates_by_group = Vec::with_capacity(per_leaf_alternatives.len());
    for (group_index, alternatives) in per_leaf_alternatives.iter().enumerate() {
        let terminal = steiner_graph.add_node();
        tree.nodes.push(false);
        tree.terminals.push(terminal);

        let mut group_candidates = Vec::with_capacity(alternatives.len());
        for (candidate_index, candidate) in alternatives.iter().enumerate() {
            let candidate_tree = candidate.get_tree(graph)?;
            let mut path = Vec::new();
            let endpoint = ingest_tree_nodes(
                graph,
                &mut steiner_graph,
                &mut node_ids,
                0,
                &candidate_tree.root.children,
                &mut path,
                None,
            )
            .unwrap_or(0);
            let requirement_cost = calculate_requirement_costs(graph, &candidate_tree.root);
            steiner_graph.add_edge(
                endpoint,
                terminal,
                requirement_cost,
                Some((group_index, candidate_index)),
            );
            group_candidates.push(candidate_tree);
        }
        candidates_by_group.push(group_candidates);
    }

    tree.edges.resize(steiner_graph.edges.len(), false);
    tree.nodes.resize(steiner_graph.incoming.len(), false);

    Ok(SteinerBuildResult {
        graph: steiner_graph,
        tree,
        candidates: candidates_by_group,
    })
}

fn ingest_tree_nodes(
    graph: &Graph,
    steiner_graph: &mut SteinerGraph,
    node_ids: &mut FxHashMap<SteinerNodeKey, SteinerNodeId>,
    parent_id: SteinerNodeId,
    nodes: &[Rc<QueryTreeNode>],
    path: &mut Vec<NodeFingerprint>,
    mut initial_tree: Option<&mut SteinerTree>,
) -> Option<SteinerNodeId> {
    let mut endpoint = None;
    for node in nodes {
        path.push(NodeFingerprint::from_node(node));
        let key = SteinerNodeKey {
            context: NodeContext::Child,
            path: path.clone(),
        };
        let node_id = match node_ids.get(&key).copied() {
            Some(node_id) => node_id,
            None => {
                let node_id = steiner_graph.add_node();
                node_ids.insert(key, node_id);
                if let Some(tree) = initial_tree.as_deref_mut() {
                    tree.nodes.resize(steiner_graph.incoming.len(), false);
                }
                node_id
            }
        };

        let edge_id =
            steiner_graph.add_edge(parent_id, node_id, node_edge_weight(graph, node), None);
        if let Some(tree) = initial_tree.as_deref_mut() {
            tree.nodes.resize(steiner_graph.incoming.len(), false);
            tree.edges.resize(steiner_graph.edges.len(), false);
            tree.nodes[node_id] = true;
            tree.edges[edge_id] = true;
        }

        endpoint = ingest_tree_nodes(
            graph,
            steiner_graph,
            node_ids,
            node_id,
            &node.children,
            path,
            initial_tree.as_deref_mut(),
        )
        .or(Some(node_id));

        path.pop();
    }
    endpoint
}

fn node_edge_weight(graph: &Graph, node: &QueryTreeNode) -> u64 {
    FIELD_COST + edge_cost(graph, node)
}

fn calculate_requirement_costs(graph: &Graph, node: &QueryTreeNode) -> u64 {
    let own_requirements = node
        .requirements
        .iter()
        .map(|requirement| CROSS_SUBGRAPH_COST + calculate_cost_of_tree(graph, requirement))
        .sum::<u64>();
    let child_requirements = node
        .children
        .iter()
        .map(|child| calculate_requirement_costs(graph, child))
        .sum::<u64>();

    own_requirements + child_requirements
}

fn solve_steiner_tree(graph: &SteinerGraph, tree: &mut SteinerTree) -> Result<(), QueryPlanError> {
    while tree.terminals.iter().any(|terminal| !tree.nodes[*terminal]) {
        if !run_flac_once(graph, tree) {
            return Err(QueryPlanError::EmptyPlan);
        }
    }

    Ok(())
}

fn run_flac_once(graph: &SteinerGraph, tree: &mut SteinerTree) -> bool {
    let terminals_count = tree.terminals.len();
    let mut flow = FlowState {
        saturated_edges: vec![false; graph.edges.len()],
        marked_edges: vec![false; graph.edges.len()],
        node_feeding_terminals: vec![
            FixedBitSet::with_capacity(terminals_count);
            graph.incoming.len()
        ],
    };
    let mut queue = PriorityQueue::<SteinerEdgeId, Reverse<u64>>::new();
    let mut added_any = false;

    for (terminal_index, terminal) in tree.terminals.iter().copied().enumerate() {
        if tree.nodes[terminal] {
            continue;
        }
        flow.node_feeding_terminals[terminal].insert(terminal_index);
        enqueue_incoming_edges(graph, &flow, terminal, &mut queue);
    }

    while let Some((edge_id, Reverse(priority))) = queue.pop() {
        if flow.marked_edges[edge_id] {
            continue;
        }

        let edge = &graph.edges[edge_id];
        let Some(current_priority) = edge_priority(edge, &flow.node_feeding_terminals[edge.to])
        else {
            continue;
        };
        if current_priority != priority {
            queue.push(edge_id, Reverse(current_priority));
            continue;
        }

        if !flow.node_feeding_terminals[edge.from]
            .is_disjoint(&flow.node_feeding_terminals[edge.to])
        {
            flow.marked_edges[edge_id] = true;
            continue;
        }

        flow.marked_edges[edge_id] = true;
        flow.saturated_edges[edge_id] = true;

        if tree.nodes[edge.from] {
            let required_terminals = flow.node_feeding_terminals[edge.to].clone();
            add_saturated_subtree(graph, tree, &flow, edge_id, &required_terminals);
            added_any = true;
            continue;
        }

        let source_feeding = flow.node_feeding_terminals[edge.to].clone();
        flow.node_feeding_terminals[edge.from].union_with(&source_feeding);
        enqueue_incoming_edges(graph, &flow, edge.from, &mut queue);
    }

    added_any
}

fn enqueue_incoming_edges(
    graph: &SteinerGraph,
    flow: &FlowState,
    node_id: SteinerNodeId,
    queue: &mut PriorityQueue<SteinerEdgeId, Reverse<u64>>,
) {
    for edge_id in &graph.incoming[node_id] {
        if flow.marked_edges[*edge_id] {
            continue;
        }
        let edge = &graph.edges[*edge_id];
        if let Some(priority) = edge_priority(edge, &flow.node_feeding_terminals[edge.to]) {
            queue.push(*edge_id, Reverse(priority));
        }
    }
}

fn edge_priority(edge: &SteinerEdge, feeding_terminals: &FixedBitSet) -> Option<u64> {
    let flow_rate = feeding_terminals.count_ones(..) as u64;
    (flow_rate > 0).then(|| edge.weight.saturating_mul(1_000_000) / flow_rate)
}

fn add_saturated_subtree(
    graph: &SteinerGraph,
    tree: &mut SteinerTree,
    flow: &FlowState,
    root_edge_id: SteinerEdgeId,
    required_terminals: &FixedBitSet,
) {
    let mut stack = vec![root_edge_id];

    while let Some(edge_id) = stack.pop() {
        if tree.edges[edge_id] {
            continue;
        }

        let edge = &graph.edges[edge_id];
        tree.nodes[edge.from] = true;
        tree.nodes[edge.to] = true;
        tree.edges[edge_id] = true;

        for next_edge_id in &graph.outgoing[edge.to] {
            let next_edge = &graph.edges[*next_edge_id];
            if next_edge.from == edge.to
                && flow.saturated_edges[*next_edge_id]
                && !flow.node_feeding_terminals[next_edge.to].is_disjoint(required_terminals)
            {
                stack.push(*next_edge_id);
            }
        }
    }
}

fn selected_candidates(
    graph: &SteinerGraph,
    tree: &SteinerTree,
    groups_count: usize,
) -> Result<Vec<usize>, QueryPlanError> {
    let mut selected = vec![None; groups_count];
    for (edge_id, edge) in graph.edges.iter().enumerate() {
        if !tree.edges[edge_id] {
            continue;
        }
        let Some((group_index, candidate_index)) = edge.candidate else {
            continue;
        };
        selected[group_index].get_or_insert(candidate_index);
    }

    selected
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or(QueryPlanError::EmptyPlan)
}

fn combination_count_under_limit(alternatives: &[Alternatives], limit: usize) -> bool {
    alternatives
        .iter()
        .try_fold(1_usize, |acc, group| {
            acc.checked_mul(group.len()).filter(|count| *count <= limit)
        })
        .is_some()
}

struct ExactSearchState {
    best_cost: u64,
    best_tree: Option<QueryTree>,
}

fn find_exact_plan(
    graph: &Graph,
    base_tree: Option<QueryTree>,
    per_leaf_alternatives: &[Alternatives],
    cancellation_token: &CancellationToken,
) -> Result<QueryTree, QueryPlanError> {
    let base_cost = base_tree
        .as_ref()
        .map(|tree| calculate_cost_of_tree(graph, &tree.root))
        .unwrap_or(0);
    let mut state = ExactSearchState {
        best_cost: u64::MAX,
        best_tree: None,
    };

    explore_exact_plans(
        graph,
        per_leaf_alternatives,
        0,
        base_tree,
        base_cost,
        cancellation_token,
        &mut state,
    )?;

    state.best_tree.ok_or(QueryPlanError::EmptyPlan)
}

fn explore_exact_plans(
    graph: &Graph,
    per_leaf_alternatives: &[Alternatives],
    group_index: usize,
    current_tree: Option<QueryTree>,
    current_cost: u64,
    cancellation_token: &CancellationToken,
    state: &mut ExactSearchState,
) -> Result<(), QueryPlanError> {
    cancellation_token.bail_if_cancelled()?;

    if group_index == per_leaf_alternatives.len() {
        if current_cost < state.best_cost {
            state.best_cost = current_cost;
            state.best_tree = current_tree;
        }
        return Ok(());
    }

    for candidate in &per_leaf_alternatives[group_index] {
        let candidate_tree = candidate.get_tree(graph)?;
        let mut next_tree = current_tree.clone();
        match next_tree.as_mut() {
            Some(tree) => Rc::make_mut(&mut tree.root).merge_nodes(&candidate_tree.root),
            None => next_tree = Some(candidate_tree),
        }

        let next_cost = next_tree
            .as_ref()
            .map(|tree| calculate_cost_of_tree(graph, &tree.root))
            .unwrap_or(0);
        if next_cost >= state.best_cost {
            continue;
        }

        explore_exact_plans(
            graph,
            per_leaf_alternatives,
            group_index + 1,
            next_tree,
            next_cost,
            cancellation_token,
            state,
        )?;
    }

    Ok(())
}

pub fn find_best_combination(
    graph: &Graph,
    operation: ResolvedOperation,
    cancellation_token: &CancellationToken,
) -> Result<QueryTree, QueryPlanError> {
    if operation.root_field_groups.is_empty()
        || operation
            .root_field_groups
            .iter()
            .any(|paths_to_leafs| paths_to_leafs.iter().any(Vec::is_empty))
    {
        return Err(QueryPlanError::EmptyPlan);
    }

    let per_leaf_alternatives = prepare_alternatives(operation);
    if per_leaf_alternatives.is_empty() {
        return Err(QueryPlanError::EmptyPlan);
    }

    // Merge fields with only one candidate
    let (base_tree, per_leaf_alternatives) =
        merge_singleton_alternatives(graph, per_leaf_alternatives)?;

    // If all fields were singletons, we are done.
    if per_leaf_alternatives.is_empty() {
        return base_tree.ok_or(QueryPlanError::EmptyPlan);
    }

    // // TODO: it's cheating...
    // if combination_count_under_limit(&per_leaf_alternatives, EXACT_COMBINATION_LIMIT) {
    //     return find_exact_plan(graph, base_tree, &per_leaf_alternatives, cancellation_token);
    // }

    let mut steiner = build_steiner_graph(graph, base_tree.as_ref(), &per_leaf_alternatives)?;
    solve_steiner_tree(&steiner.graph, &mut steiner.tree)?;
    let selected = selected_candidates(&steiner.graph, &steiner.tree, per_leaf_alternatives.len())?;

    let mut current_tree = base_tree;
    for (group_index, candidate_index) in selected.into_iter().enumerate() {
        cancellation_token.bail_if_cancelled()?;
        let next_tree = steiner.candidates[group_index][candidate_index].clone();
        match current_tree.as_mut() {
            Some(tree) => Rc::make_mut(&mut tree.root).merge_nodes(&next_tree.root),
            None => current_tree = Some(next_tree),
        }
    }

    current_tree.ok_or(QueryPlanError::EmptyPlan)
}

/// Calculate the total cost of a query tree node and all its children.
#[inline(always)]
fn calculate_cost_of_tree(graph: &Graph, node: &QueryTreeNode) -> u64 {
    let mut current_cost = FIELD_COST;

    // Add cost for each child node
    for child in &node.children {
        if edge_cost(graph, child) > 0 {
            // If this child crosses into a different subgraph
            current_cost += CROSS_SUBGRAPH_COST;
        }

        current_cost += calculate_cost_of_tree(graph, child);
    }

    for requirement in &node.requirements {
        current_cost += CROSS_SUBGRAPH_COST;
        current_cost += calculate_cost_of_tree(graph, requirement);
    }

    current_cost
}

#[inline(always)]
fn edge_cost(graph: &Graph, node: &QueryTreeNode) -> u64 {
    if node.edge_from_parent.is_some_and(|edge_index| {
        matches!(
            graph.edge(edge_index).expect("edge should exist"),
            Edge::SubgraphEntrypoint { .. }
        )
    }) {
        CROSS_SUBGRAPH_COST
    } else {
        0
    }
}
