//! This module solves the problem of finding the "best" execution plan for a
//! query that can be satisfied in many different ways by various subgraphs.
//!
//! The "best" plan is defined by a cost model that penalizes crossing
//! subgraph boundaries, and favors plans that are "local" to a single subgraph.
//!
//! Trying every combination is exponentially slow (we were there...), as the number of possible
//! plans is the product of the number of choices for each part of the query. So the plan comes
//! from two cheap passes instead:
//!
//! 1.  The query is broken down into independent decision points.
//!     For each point, there is a set of `Alternatives`, and each alternative is a
//!     `Candidate` plan fragment. The goal is to pick exactly one `Candidate` from
//!     each set of `Alternatives`. Leaves with one way to get them go in a `BasePlan`.
//!
//! 2.  A greedy pass (`Search::greedy`) goes a leaf at a time, fewest choices first, and takes
//!     the candidate that adds the least to the plan so far, the cheapest on its own on a tie.
//!
//! 3.  Going a leaf at a time, greedy can't see that one fetch could serve many leaves, so a
//!     second plan prices each hop by the leaves that could share it (`Search::by_shares`),
//!     and replaces greedy's when it makes fewer fetches. It's made twice, with hops told apart
//!     by the edges that lead to them and by the fetch they end up in (`HopKey`), as neither
//!     is always better.
//!
//! A branch-and-bound search used to run after greedy, but it never got past its first check:
//! its bound was the sum of each leaf's cheapest cost on its own, and merging a candidate into
//! a plan never adds more than that, so greedy's plan always met the bound.
//!
//! A `Candidate` is costed and merged from its path, its `QueryTree` is built only when needed
//! (`OnceCell`), and what merging adds is costed without re-costing the whole tree (`Costs`).

use ahash::{HashMap, HashMapExt, HashSet, HashSetExt};
use petgraph::graph::NodeIndex;
use rustc_hash::FxHasher;
use std::{
    cell::OnceCell,
    hash::{Hash, Hasher},
    rc::Rc,
};

use crate::query_planner::{
    graph::{edge::Edge, error::GraphError, Graph},
    planner::{
        error::QueryPlanError,
        tree::{
            query_tree::QueryTree,
            query_tree_node::{merge_query_tree_node_list, MutationFieldPosition, QueryTreeNode},
        },
        walker::{
            path::{OperationPath, PathSegment},
            ResolvedOperation,
        },
    },
    state::supergraph_state::OperationKind,
    utils::cancellation::CancellationToken,
};

type PathAndPosition<'graph> = (OperationPath<'graph>, MutationFieldPosition);

/// The high-penalty cost for crossing a subgraph boundary or satisfying a requirement.
/// This is the primary driver of the optimization, encouraging plans with fewer subgraphs.
const CROSS_SUBGRAPH_COST: u64 = 1000;
const FIELD_COST: u64 = 1;

pub fn find_best_combination(
    graph: &Graph<'_>,
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

    let leaves = Leaves::of(operation);
    if leaves.singles.is_empty() && leaves.groups.is_empty() {
        return Err(QueryPlanError::EmptyPlan);
    }

    // with no choices to make, there's nothing to price hops for
    let with_hops = !leaves.groups.is_empty();
    let base = BasePlan::merge(graph, leaves.singles, with_hops, cancellation_token)?;
    let groups = leaves
        .groups
        .into_iter()
        .map(|paths| Candidate::alternatives(graph, paths))
        .collect::<Result<Vec<_>, _>>()?;

    let mut search = Search {
        graph,
        groups: &groups,
        best_tree: None,
    };
    search.greedy(base.tree.clone());
    if with_hops {
        search.by_shares(&base, HopKey::Edges);
        search.by_shares(&base, HopKey::Fetch);
    }

    search.best_tree.ok_or(QueryPlanError::EmptyPlan)
}

/// The ways to get each leaf of the query.
struct Leaves<'graph> {
    singles: Vec<PathAndPosition<'graph>>,
    /// Fewest ways first.
    groups: Vec<Vec<PathAndPosition<'graph>>>,
}

impl<'graph> Leaves<'graph> {
    fn of(operation: ResolvedOperation<'graph>) -> Self {
        let is_mutation = matches!(operation.operation_kind, OperationKind::Mutation);
        let mut singles = Vec::new();
        let mut groups = Vec::new();

        for (index, root_field_options) in operation.root_field_groups.into_iter().enumerate() {
            let mutation_field_position: MutationFieldPosition = is_mutation.then_some(index);

            for mut paths_to_leaf in root_field_options {
                if paths_to_leaf.len() == 1 {
                    let path = paths_to_leaf.pop().expect("one path");
                    singles.push((path, mutation_field_position));
                    continue;
                }
                groups.push(
                    paths_to_leaf
                        .into_iter()
                        .map(|path| (path, mutation_field_position))
                        .collect::<Vec<_>>(),
                );
            }
        }

        groups.sort_by_key(|alternatives| alternatives.len());

        Self { singles, groups }
    }
}

/// The leaves with one way to get them are in every plan, so they're merged once, in place,
/// instead of copying the tree on every search step.
struct BasePlan {
    tree: Option<QueryTree>,
    /// The hops of the merged leaves by both keys, see `Candidate::hops`. The two kinds of key
    /// are hashes of different things, so they don't mix up.
    hops: HashSet<u64>,
}

impl BasePlan {
    fn merge(
        graph: &Graph<'_>,
        singles: Vec<PathAndPosition<'_>>,
        with_hops: bool,
        cancellation_token: &CancellationToken,
    ) -> Result<Self, QueryPlanError> {
        let mut base = BasePlan {
            tree: None,
            hops: HashSet::new(),
        };

        for (path, mutation_field_position) in singles {
            cancellation_token.bail_if_cancelled()?;
            let candidate = Candidate::new(graph, path, mutation_field_position)?;
            if with_hops {
                base.hops.extend(candidate.hops(graph, HopKey::Edges)?);
                base.hops.extend(candidate.hops(graph, HopKey::Fetch)?);
            }
            match base.tree.as_mut() {
                Some(tree) => candidate.merge_into(graph, Rc::make_mut(&mut tree.root))?,
                None => base.tree = Some(candidate.tree(graph)?),
            }
        }

        Ok(base)
    }
}

/// One way to get a leaf. Costed and merged from its path, its `QueryTree` is built only
/// when the search needs it.
struct Candidate<'graph> {
    path: OperationPath<'graph>,
    mutation_field_position: MutationFieldPosition,
    /// `@fromContext` paths always go through their `QueryTree`.
    uses_context: bool,
    cost: u64,
    tree: OnceCell<QueryTree>,
}

/// What makes two hops the same one, for `Search::by_shares`.
#[derive(Clone, Copy)]
enum HopKey {
    /// The edges that lead to it, so paths that would share it in the tree share it.
    Edges,
    /// The fetch it ends up in: the subgraph it goes to and the response path it's at, the way
    /// the fetch graph merges calls. So `... on T { name }` and `... on U { name }` both from one
    /// subgraph share it, though they're apart in the tree.
    Fetch,
}

/// The ways to get one leaf, the plan picks one.
type Alternatives<'graph> = Vec<Candidate<'graph>>;

impl<'graph> Candidate<'graph> {
    fn new(
        graph: &Graph<'_>,
        path: OperationPath<'graph>,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<Self, GraphError> {
        let uses_context = path
            .segments_rev()
            .any(|segment| !segment.context_requirements().is_empty());
        let tree = OnceCell::new();
        let cost = if uses_context {
            let built = QueryTree::from_path(graph, &path, mutation_field_position)?;
            let cost = Costs(graph).tree(&built.root);
            let _ = tree.set(built);
            cost
        } else {
            FIELD_COST + Costs(graph).of_segments(&path.get_segments())?
        };
        Ok(Self {
            path,
            mutation_field_position,
            uses_context,
            cost,
            tree,
        })
    }

    /// Cheapest first, so good plans show up sooner.
    fn alternatives(
        graph: &Graph<'_>,
        paths: Vec<PathAndPosition<'graph>>,
    ) -> Result<Alternatives<'graph>, GraphError> {
        let mut alternatives = paths
            .into_iter()
            .map(|(path, position)| Candidate::new(graph, path, position))
            .collect::<Result<Alternatives, _>>()?;
        alternatives.sort_by_key(|candidate| candidate.cost);
        Ok(alternatives)
    }

    /// The trips to other subgraphs on this path, the segments that cost a crossing: going into a
    /// subgraph, or an entity call for a requirement, each keyed by `key`. Conditions on the way
    /// aren't in either key, it's only for pricing, the plan is still costed on the tree.
    fn hops(&self, graph: &Graph<'_>, key: HopKey) -> Result<Vec<u64>, GraphError> {
        let hash = |value: &dyn Fn(&mut FxHasher)| {
            let mut hasher = FxHasher::default();
            value(&mut hasher);
            hasher.finish()
        };
        let position = self.mutation_field_position;
        let mut hops = Vec::new();
        // the edges so far for `HopKey::Edges`, the response path so far for `HopKey::Fetch`
        let mut prefix = 0;
        for segment in self.path.get_segments() {
            let edge = graph.edge(segment.edge_index)?;
            match (key, edge) {
                (HopKey::Edges, _) => {
                    let edge = segment.edge_index.index();
                    prefix = hash(&|h| (prefix, edge, position).hash(h));
                }
                (HopKey::Fetch, Edge::FieldMove(field)) => {
                    let name = segment
                        .selection_attributes
                        .as_ref()
                        .and_then(|attributes| attributes.alias.as_deref())
                        .unwrap_or(field.name);
                    prefix = hash(&|h| (prefix, name).hash(h));
                }
                (HopKey::Fetch, _) => {}
            }
            if segment.requirement_tree.is_none()
                && !matches!(edge, Edge::SubgraphEntrypoint { .. })
            {
                continue;
            }
            hops.push(match key {
                HopKey::Edges => prefix,
                HopKey::Fetch => {
                    let subgraph = graph
                        .node(graph.get_edge_tail(&segment.edge_index)?)?
                        .subgraph_index();
                    // a @requires field's own call is apart from the entity calls at its parent
                    let requires = matches!(edge, Edge::FieldMove(_));
                    hash(&|h| (prefix, subgraph, requires, position).hash(h))
                }
            });
        }
        Ok(hops)
    }

    fn tree(&self, graph: &Graph<'_>) -> Result<QueryTree, GraphError> {
        if let Some(tree) = self.tree.get() {
            return Ok(tree.clone());
        }
        let tree = QueryTree::from_path(graph, &self.path, self.mutation_field_position)?;
        Ok(self.tree.get_or_init(|| tree).clone())
    }

    /// How much merging this candidate into `root` adds to its cost.
    fn added_cost(&self, graph: &Graph<'_>, root: &QueryTreeNode) -> Result<u64, GraphError> {
        if self.uses_context {
            return Ok(Costs(graph).added_by_merge(root, &self.tree(graph)?.root));
        }
        Costs(graph).added_by_path(
            root,
            &self.path.get_segments(),
            self.mutation_field_position,
        )
    }

    fn merge_into(&self, graph: &Graph<'_>, root: &mut QueryTreeNode) -> Result<(), GraphError> {
        if self.uses_context {
            root.merge_nodes(&self.tree(graph)?.root);
            return Ok(());
        }
        merge_path(
            graph,
            root,
            &self.path.get_segments(),
            self.mutation_field_position,
        )
    }
}

/// Picks one candidate per group.
struct Search<'a, 'graph> {
    graph: &'a Graph<'a>,
    groups: &'a [Alternatives<'graph>],
    best_tree: Option<QueryTree>,
}

impl Search<'_, '_> {
    /// A fast, good-enough plan: per leaf, the candidate that adds the least.
    fn greedy(&mut self, mut current_tree: Option<QueryTree>) {
        let graph = self.graph;
        for alternatives in self.groups {
            let mut best_delta = u64::MAX;
            let mut best_candidate: Option<&Candidate> = None;

            for candidate in alternatives {
                let delta = match current_tree.as_ref() {
                    Some(tree) => match candidate.added_cost(graph, &tree.root) {
                        Ok(added_cost) => added_cost,
                        Err(_) => continue,
                    },
                    None => candidate.cost,
                };

                if delta < best_delta {
                    best_delta = delta;
                    best_candidate = Some(candidate);
                }
            }

            let Some(candidate) = best_candidate else {
                return;
            };
            if self.add(&mut current_tree, candidate).is_none() {
                return;
            }
        }

        self.best_tree = current_tree;
    }

    /// A second plan, for what greedy can't see. Going a leaf at a time, greedy opens the fetch
    /// that's cheapest for that leaf, even when another one could serve many leaves. Here each
    /// hop's crossing is split between the leaves that could use it (hops the plan already has
    /// are free), and every leaf takes its cheapest candidate at those prices.
    ///
    /// It replaces the plan so far when it has fewer fetches (`Costs::by_fetches`), not by the
    /// tree cost, which charges a crossing per requirement: a plan cheaper by that can still make
    /// a fetch more.
    fn by_shares(&mut self, base: &BasePlan, key: HopKey) {
        let graph = self.graph;
        let Ok(hops) = self
            .groups
            .iter()
            .map(|alternatives| {
                alternatives
                    .iter()
                    .map(|candidate| candidate.hops(graph, key))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()
        else {
            return;
        };

        // How many leaves could use each hop
        let mut users: HashMap<u64, u32> = HashMap::new();
        let mut seen = HashSet::new();
        for leaf in &hops {
            seen.clear();
            for &hop in leaf.iter().flatten() {
                if seen.insert(hop) {
                    *users.entry(hop).or_default() += 1;
                }
            }
        }
        let crossing = CROSS_SUBGRAPH_COST as f64;
        let price = |candidate: &Candidate, hops: &[u64]| {
            let discount: f64 = hops
                .iter()
                .map(|hop| match base.hops.contains(hop) {
                    true => crossing,
                    false => crossing - crossing / f64::from(users[hop]),
                })
                .sum();
            candidate.cost as f64 - discount
        };

        let mut plan = base.tree.clone();
        for (alternatives, hops) in self.groups.iter().zip(&hops) {
            // the first one on a tie, the cheapest on its own
            let Some((candidate, _)) = alternatives
                .iter()
                .zip(hops)
                .min_by(|(a, a_hops), (b, b_hops)| price(a, a_hops).total_cmp(&price(b, b_hops)))
            else {
                return;
            };
            if self.add(&mut plan, candidate).is_none() {
                return;
            }
        }

        let Some(tree) = plan else {
            return;
        };
        let costs = Costs(graph);
        let fewer_fetches = self
            .best_tree
            .as_ref()
            .is_none_or(|best| costs.by_fetches(&tree.root) < costs.by_fetches(&best.root));
        if fewer_fetches {
            self.best_tree = Some(tree);
        }
    }

    /// Merges `candidate` into the plan so far. `None` if the graph is missing a node or edge
    /// of its path.
    fn add(&self, plan: &mut Option<QueryTree>, candidate: &Candidate) -> Option<()> {
        match plan.as_mut() {
            Some(tree) => candidate
                .merge_into(self.graph, Rc::make_mut(&mut tree.root))
                .ok(),
            None => {
                *plan = Some(candidate.tree(self.graph).ok()?);
                Some(())
            }
        }
    }
}

/// The child of `node` made from `segment`, compared like `QueryTreeNode::eq`.
fn find_path_child(
    node: &QueryTreeNode,
    tail_node_index: NodeIndex,
    segment: &PathSegment,
    position: MutationFieldPosition,
) -> Option<usize> {
    node.children.iter().position(|child| {
        child.node_index == tail_node_index
            && child.edge_from_parent == Some(segment.edge_index)
            && child.selection_attributes == segment.selection_attributes
            && child.mutation_field_position == position
            && child.condition == segment.condition
            && child.context_coordinate().is_none()
    })
}

/// Merges a path into `root` without building its `QueryTree` first. Not for `@fromContext`.
fn merge_path(
    graph: &Graph<'_>,
    root: &mut QueryTreeNode,
    segments: &[&PathSegment],
    mutation_field_position: MutationFieldPosition,
) -> Result<(), GraphError> {
    let mut node = root;
    for (index, segment) in segments.iter().enumerate() {
        let tail_node_index = graph.get_edge_tail(&segment.edge_index)?;
        let position = mutation_field_position.filter(|_| index == 0);

        let Some(existing) = find_path_child(node, tail_node_index, segment, position) else {
            let rest =
                QueryTreeNode::from_path_segment_sequences(graph, segments, index, position)?
                    .expect("a segment is left");
            node.children.push(rest);
            return Ok(());
        };

        let child = Rc::make_mut(&mut node.children[existing]);
        if let Some(requirement) = &segment.requirement_tree {
            merge_query_tree_node_list(&mut child.requirements, std::slice::from_ref(requirement));
        }
        node = child;
    }
    Ok(())
}

/// What a tree costs, and what a merge adds to it.
#[derive(Clone, Copy)]
struct Costs<'a>(&'a Graph<'a>);

/// A fetch the way the fetch graph makes it: a subgraph's root fetch, or the entity calls to a
/// subgraph after as many other entity calls. The fetch graph batches calls to a subgraph across
/// response paths, so it's per depth, not per object.
type FetchKey = (bool, Option<u16>, u32, MutationFieldPosition);

/// Walks a tree for `Costs::by_fetches`, with how many entity calls lead to each node.
struct FetchWalk<'a> {
    graph: &'a Graph<'a>,
    nodes: u64,
    /// The nodes that resolve context
    crossings: u64,
    fetches: HashSet<FetchKey>,
}

impl FetchWalk<'_> {
    fn node(&mut self, node: &QueryTreeNode, depth: u32, position: MutationFieldPosition) {
        self.nodes += 1;
        if !node.resolved_context_requirements().is_empty() {
            self.crossings += 1;
        }
        let position = node.mutation_field_position.or(position);
        let subgraph = self
            .graph
            .node(node.node_index)
            .ok()
            .and_then(|node| node.subgraph_index());
        let entrypoint = node.edge_from_parent.is_some_and(|edge| {
            matches!(self.graph.edge(edge), Ok(Edge::SubgraphEntrypoint { .. }))
        });
        if entrypoint {
            self.fetches.insert((false, subgraph, 0, position));
        }
        for requirement in &node.requirements {
            self.fetches.insert((true, subgraph, depth, position));
            self.node(requirement, depth, position);
        }
        for context in node.context_requirements() {
            self.node(context, depth, position);
        }
        let depth = depth + u32::from(!node.requirements.is_empty());
        for child in &node.children {
            self.node(child, depth, position);
        }
    }
}

impl Costs<'_> {
    fn tree(self, node: &QueryTreeNode) -> u64 {
        let mut current_cost = FIELD_COST;

        for child in &node.children {
            if self.edge(child) > 0 {
                current_cost += CROSS_SUBGRAPH_COST;
            }

            current_cost += self.tree(child);
        }

        for requirement in &node.requirements {
            current_cost += CROSS_SUBGRAPH_COST;
            current_cost += self.tree(requirement);
        }

        for requirement in node.context_requirements() {
            current_cost += self.tree(requirement);
        }
        if !node.resolved_context_requirements().is_empty() {
            current_cost += CROSS_SUBGRAPH_COST;
        }

        current_cost
    }

    /// What a whole plan costs counting fetches close to how the fetch graph makes them: a field
    /// per node, a crossing per subgraph's root fetch, one per subgraph's entity calls at a depth
    /// (however many nodes need them, see `FetchKey`), and one per node that resolves context.
    // ponytail: depth stands in for the batching the fetch graph does, it's only exact when
    // calls at the same depth don't depend on each other. Good enough to compare two plans.
    fn by_fetches(self, root: &QueryTreeNode) -> u64 {
        let mut walk = FetchWalk {
            graph: self.0,
            nodes: 0,
            crossings: 0,
            fetches: HashSet::new(),
        };
        walk.node(root, 0, None);
        walk.nodes * FIELD_COST + (walk.fetches.len() as u64 + walk.crossings) * CROSS_SUBGRAPH_COST
    }

    /// What getting to `node` from its parent costs.
    fn edge(self, node: &QueryTreeNode) -> u64 {
        if node.edge_from_parent.is_some_and(|edge_index| {
            matches!(
                self.0.edge(edge_index).expect("edge should exist"),
                Edge::SubgraphEntrypoint { .. }
            )
        }) {
            CROSS_SUBGRAPH_COST
        } else {
            0
        }
    }

    /// What these segments cost as nodes, like `tree` without the root.
    fn of_segments(self, segments: &[&PathSegment]) -> Result<u64, GraphError> {
        let mut cost = 0;
        for segment in segments {
            cost += FIELD_COST;
            if matches!(
                self.0.edge(segment.edge_index)?,
                Edge::SubgraphEntrypoint { .. }
            ) {
                cost += CROSS_SUBGRAPH_COST;
            }
            if let Some(requirement) = &segment.requirement_tree {
                cost += CROSS_SUBGRAPH_COST + self.tree(requirement);
            }
        }
        Ok(cost)
    }

    /// What `merge_path` would add to the cost of `root`.
    fn added_by_path(
        self,
        root: &QueryTreeNode,
        segments: &[&PathSegment],
        mutation_field_position: MutationFieldPosition,
    ) -> Result<u64, GraphError> {
        let mut added_cost = 0;
        let mut node = root;
        for (index, segment) in segments.iter().enumerate() {
            let tail_node_index = self.0.get_edge_tail(&segment.edge_index)?;
            let position = mutation_field_position.filter(|_| index == 0);

            let Some(existing) = find_path_child(node, tail_node_index, segment, position) else {
                return Ok(added_cost + self.of_segments(&segments[index..])?);
            };

            let child = &node.children[existing];
            if let Some(requirement) = &segment.requirement_tree {
                added_cost += self.added_by_list(
                    &child.requirements,
                    std::slice::from_ref(requirement),
                    true,
                );
            }
            node = child;
        }
        Ok(added_cost)
    }

    /// How much `source` adds when merged into `target`, same as `tree` after the merge minus
    /// before, without costing the whole tree.
    fn added_by_merge(self, target: &QueryTreeNode, source: &QueryTreeNode) -> u64 {
        // context trees merge like requirements, but `tree` charges no hop for them,
        // just one hop for having any resolved context at all
        let context_cost: u64 = source
            .context_requirements()
            .iter()
            .map(|source_node| {
                match target
                    .context_requirements()
                    .iter()
                    .find(|target_node| target_node.as_ref() == source_node.as_ref())
                {
                    Some(target_node) => self.added_by_merge(target_node, source_node),
                    None => self.tree(source_node),
                }
            })
            .sum();
        let resolved_context_cost = if target.resolved_context_requirements().is_empty()
            && !source.resolved_context_requirements().is_empty()
        {
            CROSS_SUBGRAPH_COST
        } else {
            0
        };

        self.added_by_list(&target.children, &source.children, false)
            + self.added_by_list(&target.requirements, &source.requirements, true)
            + context_cost
            + resolved_context_cost
    }

    fn added_by_list(
        self,
        target_list: &[Rc<QueryTreeNode>],
        source_list: &[Rc<QueryTreeNode>],
        is_requirement: bool,
    ) -> u64 {
        source_list
            .iter()
            .map(|source_node| {
                if let Some(target_node) = target_list
                    .iter()
                    .find(|target_node| target_node.as_ref() == source_node.as_ref())
                {
                    self.added_by_merge(target_node, source_node)
                } else if is_requirement {
                    CROSS_SUBGRAPH_COST + self.tree(source_node)
                } else {
                    self.edge(source_node) + self.tree(source_node)
                }
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use petgraph::graph::NodeIndex;

    use super::*;
    use crate::query_planner::planner::tree::query_tree_node::NodeContext;
    use crate::query_planner::planner::walker::path::ContextRequirement;

    fn node(
        index: usize,
        children: Vec<Rc<QueryTreeNode>>,
        requirements: Vec<Rc<QueryTreeNode>>,
    ) -> Rc<QueryTreeNode> {
        Rc::new(QueryTreeNode {
            node_index: NodeIndex::new(index),
            edge_from_parent: None,
            requirements,
            context: None,
            children,
            selection_attributes: None,
            condition: None,
            mutation_field_position: None,
        })
    }

    #[test]
    fn incremental_merge_cost_matches_full_tree_cost() {
        let graph = Graph::default();
        // The graph is only consulted for edge costs. These test nodes have no graph edges.
        let costs = Costs(&graph);

        let shared_leaf = node(2, vec![], vec![]);
        let shared_requirement = node(3, vec![], vec![]);
        let target_shared = node(1, vec![shared_leaf], vec![shared_requirement]);
        let target = QueryTreeNode {
            node_index: NodeIndex::new(0),
            edge_from_parent: None,
            requirements: vec![],
            context: None,
            children: vec![target_shared],
            selection_attributes: None,
            condition: None,
            mutation_field_position: None,
        };

        let source_shared = node(
            1,
            vec![node(2, vec![], vec![]), node(4, vec![], vec![])],
            vec![node(3, vec![], vec![]), node(5, vec![], vec![])],
        );
        let source = QueryTreeNode {
            node_index: NodeIndex::new(0),
            edge_from_parent: None,
            requirements: vec![],
            context: Some(Box::new(NodeContext {
                requirements: vec![node(7, vec![], vec![])],
                coordinate: None,
                resolved_requirements: vec![Rc::new(ContextRequirement {
                    argument: 0,
                    selection: 0,
                    levels: 1,
                    segments_back: 1,
                    cost: 0,
                    tree: node(8, vec![], vec![]),
                })],
            })),
            children: vec![source_shared, node(6, vec![], vec![])],
            selection_attributes: None,
            condition: None,
            mutation_field_position: None,
        };

        let current_cost = costs.tree(&target);
        let added_cost = costs.added_by_merge(&target, &source);
        let mut merged = target.clone();
        merged.merge_nodes(&source);

        assert_eq!(added_cost, costs.tree(&merged) - current_cost);
    }
}
