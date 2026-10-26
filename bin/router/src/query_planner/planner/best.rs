//! This module solves the problem of finding the "best" execution plan for a
//! query that can be satisfied in many different ways by various subgraphs.
//!
//! The "best" plan is defined by a cost model that penalizes crossing
//! subgraph boundaries, and favors plans that are "local" to a single subgraph.
//!
//! The query is broken down into decision points, one per leaf. Each has a set of
//! `Alternatives`, and each alternative is a `Candidate`: one path that resolves the leaf.
//! The plan picks one `Candidate` per set.
//!
//! A candidate's tree is a chain, a node per path segment, plus the requirement trees hanging off
//! it. A plan is those trees merged, and costs a field per node plus a crossing per fetch it needs.
//! Fetches are counted the way the fetch graph makes them: one root fetch per subgraph, and one
//! entity call per subgraph and path of the objects it extends, however many nodes lead there.
//! ponytail: two calls with the same key where one waits on the other stay two requests, but
//! count as one here. Only the cost is off, the plan is still right. abstract has 20 of those,
//! the same in every plan, so no pick changes. If one ever does, look at what actually keeps
//! those calls apart first: same depth doesn't mean they can merge.
//! `@fromContext` costs what it did before: the nodes of the context trees (no fetch for the tree
//! itself, its fetches inside count like a requirement tree's), and a crossing for every node that
//! resolves context.
//! Each node and each fetch is an "item", interned once with its weight, and a candidate is the
//! list of its items. So a plan costs the weight of the items its candidates use, counted once.
//!
//! Picking the cheapest plan that way is a weighted set cover (which fetches cover every leaf), so
//! there's no fast way to always get it. What happens instead:
//!
//! 1. Greedy. Sets go from fewest choices to most, and each one takes the candidate that adds the
//!    least cost to the plan so far. Candidates are sorted by their own cost first, so a tie goes
//!    to the cheaper one on its own.
//! 2. A lower bound. Items every candidate of a set has are paid for sure, and the rest of an item
//!    is split between the sets that could use it. When greedy reaches the bound, it's the
//!    cheapest plan and we're done. That's the common case.
//! 3. Otherwise a Lagrangian relaxation, see `relax`: items get prices, each set takes its
//!    cheapest candidate at those prices, and the prices move a few steps. Its plan replaces
//!    greedy's when it's cheaper, and it often is by far, as it sees from the start which items
//!    many sets share.
//! 4. Then a local search, on a budget of work (not time, so the plan is the same on every run):
//!    switch one set's candidate when that's cheaper, or move every set off a fetch at once when
//!    that's cheaper. The second one finds what the first can't: a fetch shared by many sets only
//!    goes away when all of them move.
//!
//! The plan never gets worse than greedy's. Only the picked candidates become trees, merged in the
//! order of the sets, like before.

use std::{ops::Range, rc::Rc};

use ahash::{HashMap, HashMapExt};
use petgraph::graph::{EdgeIndex, NodeIndex};
use tracing::trace;

use crate::query_planner::{
    ast::merge_path::Condition,
    graph::{edge::Edge, Graph},
    planner::{
        error::QueryPlanError,
        tree::{
            query_tree::QueryTree,
            query_tree_node::{MutationFieldPosition, QueryTreeNode},
        },
        walker::{
            path::{OperationPath, PathSegment, SelectionAttributes, NO_PLANNER_ITEMS},
            ResolvedOperation,
        },
    },
    state::supergraph_state::OperationKind,
    utils::cancellation::CancellationToken,
};

/// The cost of a fetch: a root fetch, or an entity call to resolve a crossing or a requirement.
/// This is the primary driver of the optimization, encouraging plans with fewer fetches.
const CROSS_SUBGRAPH_COST: u64 = 1000;
const FIELD_COST: u64 = 1;

/// How much work the local search may do, per item a candidate lists. Counted in items looked at,
/// so the same query always gets the same plan, however busy the machine is.
const SEARCH_WORK_PER_CANDIDATE_ITEM: usize = 16;

/// One possible way to resolve a leaf of the query. There's one per path the walker found, so
/// it keeps only what rebuilds the path's segments, not the whole path.
struct Candidate {
    root_node: NodeIndex,
    last_segment: Option<Rc<PathSegment>>,
    mutation_field_position: MutationFieldPosition,
    /// The cost of the candidate's tree on its own.
    cost: u64,
    /// Where the items of a mutation's candidate are in `Items::lists`. Other candidates read
    /// theirs from their segments, see `Items::of`.
    listed: Option<Range<u32>>,
}

/// The candidates of one leaf, a set. The plan picks one candidate per set.
type Alternatives = Vec<Candidate>;

impl Candidate {
    /// The path again, minus what the walker needed (union context, cost).
    fn path(&self) -> OperationPath<'static> {
        OperationPath {
            root_node: self.root_node,
            last_segment: self.last_segment.clone(),
            cost: 0,
            union_context: None,
        }
    }

    fn build_tree(&self, graph: &Graph<'_>) -> Result<QueryTree, QueryPlanError> {
        Ok(QueryTree::from_path(
            graph,
            &self.path(),
            self.mutation_field_position,
        )?)
    }
}

/// What makes two tree nodes the same node once merged: the same parent, in the same list
/// (children or requirements), and equal the way `QueryTreeNode` compares nodes. Hashed straight
/// from the node's fields, see `ITEM_HASHER`. Only a context tree's root has a coordinate, which
/// also keeps it apart from the children and requirements of the node it hangs on.
#[allow(clippy::too_many_arguments)]
fn node_fingerprint(
    parent: u32,
    requirement: bool,
    node_index: NodeIndex,
    edge: Option<EdgeIndex>,
    attributes: Option<&SelectionAttributes>,
    mutation_field_position: MutationFieldPosition,
    condition: Option<&Condition>,
    context_coordinate: Option<(EdgeIndex, usize)>,
) -> u64 {
    let attributes = attributes.map(|attributes| (&attributes.alias, &attributes.arguments));
    ITEM_HASHER.hash_one((
        parent,
        requirement,
        node_index,
        edge,
        attributes,
        mutation_field_position,
        condition,
        context_coordinate,
    ))
}

/// The parent of the first segment's item, the plan's root.
const ROOT_ITEM: u32 = u32::MAX;

/// Items are interned by a 64-bit hash of their key, not the key, which would make the table
/// several times bigger. Two items colliding takes billions of them, and would only make a cost
/// wrong: the plan is still built from the trees. Fixed seeds, so plans don't change between runs.
const ITEM_HASHER: ahash::RandomState = ahash::RandomState::with_seeds(
    0x243f_6a88_85a3_08d3,
    0x1319_8a2e_0370_7344,
    0xa409_3822_299f_31d0,
    0x082e_fa98_ec4e_6c89,
);

/// The weight of every item, and the items of requirement trees and of mutation candidates.
struct Items {
    weights: Vec<u64>,
    /// The items of each segment's requirement tree, see `PathSegment::planner_items`.
    requirement_items: Vec<u32>,
    lists: Vec<u32>,
}

impl Items {
    /// The items of a candidate's tree, no duplicates. Read from its segments, which remember
    /// theirs, so there's no list per candidate (a mutation's candidates are the exception).
    fn of<'a>(&'a self, candidate: &'a Candidate) -> CandidateItems<'a> {
        match &candidate.listed {
            Some(range) => {
                CandidateItems::Listed(self.lists[range.start as usize..range.end as usize].iter())
            }
            None => CandidateItems::Segments {
                segment: candidate.last_segment.as_deref(),
                requirement: [].iter(),
                requirement_items: &self.requirement_items,
            },
        }
    }
}

enum CandidateItems<'a> {
    Listed(std::slice::Iter<'a, u32>),
    /// From the last segment back: each one's item, then its requirement's.
    Segments {
        segment: Option<&'a PathSegment>,
        requirement: std::slice::Iter<'a, u32>,
        requirement_items: &'a [u32],
    },
}

impl Iterator for CandidateItems<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        match self {
            CandidateItems::Listed(items) => items.next().copied(),
            CandidateItems::Segments {
                segment,
                requirement,
                requirement_items,
            } => {
                if let Some(&item) = requirement.next() {
                    return Some(item);
                }
                let current = (*segment)?;
                let [item, start, end, _] = current.planner_items.get();
                *requirement = requirement_items[start as usize..end as usize].iter();
                *segment = current.prev();
                Some(item)
            }
        }
    }
}

/// Which fetch a crossing becomes. The fetch graph gives a subgraph one root fetch, and one entity
/// call per path of the objects it extends: steps to the same subgraph at the same path get reused
/// and merged. So that's what a crossing costs, once, however many tree nodes lead to it.
#[derive(Hash)]
struct FetchKey {
    entity_call: bool,
    subgraph: Option<u16>,
    /// The response path of the objects, interned. The root fetch has none.
    path: u32,
    /// A mutation's root fields run one after another, each its own fetch.
    mutation_field_position: Option<u32>,
}

/// The response path of the plan's root.
const ROOT_PATH: u32 = u32::MAX;

/// Turns candidates into items. Paths share their prefixes, so each segment is worked out once
/// and remembered in `PathSegment::planner_items`, with the items it brings along.
struct ItemBuilder<'a, 'graph> {
    graph: &'a Graph<'graph>,
    ids: HashMap<u64, u32>,
    /// Response paths, interned: a parent path and a response key.
    paths: HashMap<u64, u32>,
    items: Items,
    /// One mutation candidate's items, before they're sorted into `items.lists`.
    scratch: Vec<u32>,
    /// The item, response path and parent's response path of each segment of the path so far,
    /// for the context trees that hang on an earlier segment's node.
    chain: Vec<[u32; 3]>,
}

impl<'a, 'graph> ItemBuilder<'a, 'graph> {
    fn new(graph: &'a Graph<'graph>) -> Self {
        Self {
            graph,
            ids: HashMap::new(),
            paths: HashMap::new(),
            items: Items {
                weights: Vec::new(),
                requirement_items: Vec::new(),
                lists: Vec::new(),
            },
            scratch: Vec::new(),
            chain: Vec::new(),
        }
    }

    fn intern(&mut self, fingerprint: u64, weight: u64) -> u32 {
        let weights = &mut self.items.weights;
        *self.ids.entry(fingerprint).or_insert_with(|| {
            weights.push(weight);
            (weights.len() - 1) as u32
        })
    }

    fn node_item(&mut self, fingerprint: u64) -> u32 {
        self.intern(fingerprint, FIELD_COST)
    }

    /// The crossing a node pays for resolving context, once however many arguments it resolves.
    fn resolved_context_item(&mut self, node: u32) -> u32 {
        self.intern(
            ITEM_HASHER.hash_one(("resolved context", node)),
            CROSS_SUBGRAPH_COST,
        )
    }

    /// The fetch an entrypoint or a requirement of a node in `node_index`'s subgraph needs.
    fn fetch_item(
        &mut self,
        entity_call: bool,
        node_index: NodeIndex,
        path: u32,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<u32, QueryPlanError> {
        let key = FetchKey {
            entity_call,
            subgraph: self.graph.node(node_index)?.subgraph_index(),
            path: if entity_call { path } else { ROOT_PATH },
            mutation_field_position: mutation_field_position.map(|position| position as u32),
        };
        Ok(self.intern(ITEM_HASHER.hash_one(&key), CROSS_SUBGRAPH_COST))
    }

    /// The response path below `edge`: one key longer when it's a field, the same otherwise.
    fn path_below(
        &mut self,
        path: u32,
        edge: &Edge<'_>,
        attributes: Option<&Rc<SelectionAttributes>>,
    ) -> u32 {
        let Edge::FieldMove(field) = edge else {
            return path;
        };
        let response_key = attributes
            .and_then(|attributes| attributes.alias.as_deref())
            .unwrap_or(field.name);
        let next = self.paths.len() as u32;
        *self
            .paths
            .entry(ITEM_HASHER.hash_one((path, response_key)))
            .or_insert(next)
    }

    /// Works out the items of the path's segments. Returns where a mutation's candidate items
    /// are listed, other candidates read theirs from the segments.
    fn candidate_items(
        &mut self,
        path: &OperationPath<'_>,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<Option<Range<u32>>, QueryPlanError> {
        let segments = path.get_segments();
        // A mutation's first segment can be shared between root fields, each with its own items,
        // so a segment can't remember them.
        let listed = mutation_field_position.is_some();
        let mut items = std::mem::take(&mut self.scratch);
        let mut chain = std::mem::take(&mut self.chain);
        chain.clear();
        let mut parent = ROOT_ITEM;
        let mut parent_path = ROOT_PATH;

        for (index, segment) in segments.iter().enumerate() {
            let cached = segment.planner_items.get();
            let (item, brought, path) = if !listed && cached != NO_PLANNER_ITEMS {
                (cached[0], cached[1]..cached[2], cached[3])
            } else {
                let position = if index == 0 {
                    mutation_field_position
                } else {
                    None
                };
                let (item, path, brought) = self.segment_items(
                    parent,
                    parent_path,
                    segment,
                    &chain,
                    position,
                    mutation_field_position,
                )?;
                if !listed {
                    segment
                        .planner_items
                        .set([item, brought.start, brought.end, path]);
                }
                (item, brought, path)
            };
            if listed {
                items.push(item);
                items.extend_from_slice(
                    &self.items.requirement_items[brought.start as usize..brought.end as usize],
                );
            }
            chain.push([item, path, parent_path]);
            parent = item;
            parent_path = path;
        }
        self.chain = chain;

        if !listed {
            self.scratch = items;
            return Ok(None);
        }
        let range = self.list(&mut items);
        self.scratch = items;
        Ok(Some(range))
    }

    /// Sorts and dedups `items` into `items.lists`, and empties it.
    fn list(&mut self, items: &mut Vec<u32>) -> Range<u32> {
        items.sort_unstable();
        items.dedup();
        let start = self.items.lists.len() as u32;
        self.items.lists.extend_from_slice(items);
        items.clear();
        start..self.items.lists.len() as u32
    }

    /// A segment's node, the items it brings along in `requirement_items` (the root fetch when it
    /// enters a subgraph, the entity call and the requirement tree when it has a requirement, the
    /// context it resolves and the context trees it hangs on earlier nodes), and its response path.
    /// `chain` has the segments before it, see `ItemBuilder::chain`.
    fn segment_items(
        &mut self,
        parent: u32,
        parent_path: u32,
        segment: &PathSegment,
        chain: &[[u32; 3]],
        position: MutationFieldPosition,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<(u32, u32, Range<u32>), QueryPlanError> {
        let node_index = self.graph.get_edge_tail(&segment.edge_index)?;
        let edge = self.graph.edge(segment.edge_index)?;
        let item = self.node_item(node_fingerprint(
            parent,
            false,
            node_index,
            Some(segment.edge_index),
            segment.selection_attributes.as_deref(),
            position,
            segment.condition.as_deref(),
            None,
        ));
        let path = self.path_below(parent_path, edge, segment.selection_attributes.as_ref());

        let start = self.items.requirement_items.len() as u32;
        if matches!(edge, Edge::SubgraphEntrypoint { .. }) {
            let fetch = self.fetch_item(false, node_index, path, mutation_field_position)?;
            self.items.requirement_items.push(fetch);
        }
        if let Some(requirement) = &segment.requirement_tree {
            // A field's requirement is read from the object the field is on.
            let fetch = self.fetch_item(true, node_index, parent_path, mutation_field_position)?;
            self.items.requirement_items.push(fetch);
            self.tree_items(
                item,
                requirement,
                true,
                parent_path,
                parent_path,
                mutation_field_position,
            )?;
        }
        if !segment.context_requirements().is_empty() {
            let resolved = self.resolved_context_item(item);
            self.items.requirement_items.push(resolved);
        }
        for requirement in segment.context_requirements() {
            // like `QueryTreeNode::from_path_segment_sequences`: the tree hangs on the node of the
            // segment `segments_back` before this one, and is dropped when there's none
            let Some(&[node, node_path, _]) = chain
                .len()
                .checked_sub(requirement.segments_back)
                .and_then(|index| chain.get(index))
            else {
                continue;
            };
            self.tree_items(
                node,
                &requirement.tree,
                false,
                node_path,
                node_path,
                mutation_field_position,
            )?;
        }
        let end = self.items.requirement_items.len() as u32;
        Ok((item, path, start..end))
    }

    /// The items of a requirement or context tree: its nodes, the fetches of the entrypoints and
    /// requirements inside it, and its own context bits.
    fn tree_items(
        &mut self,
        parent: u32,
        node: &QueryTreeNode,
        requirement: bool,
        path: u32,
        parent_path: u32,
        mutation_field_position: MutationFieldPosition,
    ) -> Result<(), QueryPlanError> {
        let item = self.node_item(node_fingerprint(
            parent,
            requirement,
            node.node_index,
            node.edge_from_parent,
            node.selection_attributes.as_deref(),
            node.mutation_field_position,
            node.condition.as_ref(),
            node.context_coordinate(),
        ));
        self.items.requirement_items.push(item);
        if !node.resolved_context_requirements().is_empty() {
            let resolved = self.resolved_context_item(item);
            self.items.requirement_items.push(resolved);
        }

        let edge = match node.edge_from_parent {
            Some(edge) => Some(self.graph.edge(edge)?),
            None => None,
        };
        if matches!(edge, Some(Edge::SubgraphEntrypoint { .. })) {
            let fetch = self.fetch_item(false, node.node_index, path, mutation_field_position)?;
            self.items.requirement_items.push(fetch);
        }
        let object_path = match edge {
            Some(Edge::FieldMove(_)) => parent_path,
            _ => path,
        };
        for requirement in &node.requirements {
            let fetch =
                self.fetch_item(true, node.node_index, object_path, mutation_field_position)?;
            self.items.requirement_items.push(fetch);
            self.tree_items(
                item,
                requirement,
                true,
                object_path,
                object_path,
                mutation_field_position,
            )?;
        }
        for context in node.context_requirements() {
            self.tree_items(item, context, false, path, path, mutation_field_position)?;
        }
        for child in &node.children {
            let child_path = match child.edge_from_parent {
                Some(edge) => {
                    let edge = self.graph.edge(edge)?;
                    self.path_below(path, edge, child.selection_attributes.as_ref())
                }
                None => path,
            };
            self.tree_items(
                item,
                child,
                false,
                child_path,
                path,
                mutation_field_position,
            )?;
        }
        Ok(())
    }
}

/// One set per leaf, fewest candidates first, and the items the candidates are made of.
fn prepare_alternatives(
    graph: &Graph<'_>,
    operation: ResolvedOperation<'_>,
) -> Result<(Vec<Alternatives>, Items), QueryPlanError> {
    let is_mutation = matches!(operation.operation_kind, OperationKind::Mutation);
    let mut groups: Vec<Alternatives> = Vec::new();
    let mut builder = ItemBuilder::new(graph);
    let mut repeatable: Vec<u32> = Vec::new();

    for (index, root_field_options) in operation.root_field_groups.into_iter().enumerate() {
        let mutation_field_position: MutationFieldPosition = is_mutation.then_some(index);

        for paths_to_leaf in root_field_options {
            // Sized up front, collecting through `Result` would grow it and leave spare capacity.
            let mut alternatives = Alternatives::with_capacity(paths_to_leaf.len());
            for path in paths_to_leaf {
                let listed = builder.candidate_items(&path, mutation_field_position)?;
                // The same context tree can hang on a node twice (the same contextual field
                // twice in the path), so then any item can repeat, not only fetches.
                let has_context =
                    std::iter::successors(path.last_segment.as_deref(), |segment| segment.prev())
                        .any(|segment| !segment.context_requirements().is_empty());
                let mut candidate = Candidate {
                    root_node: path.root_node,
                    last_segment: path.last_segment,
                    mutation_field_position,
                    cost: 0,
                    listed,
                };
                // The tree's root, then its items. A chain can need the same fetch twice (into a
                // subgraph, out, and back at the same path), and items must be counted once, so
                // such a candidate gets a deduplicated list. Without context only fetches repeat.
                repeatable.clear();
                let mut cost = FIELD_COST;
                for item in builder.items.of(&candidate) {
                    let weight = builder.items.weights[item as usize];
                    if has_context || weight >= CROSS_SUBGRAPH_COST {
                        repeatable.push(item);
                    }
                    cost += weight;
                }
                repeatable.sort_unstable();
                let repeated: u64 = repeatable
                    .windows(2)
                    .filter(|pair| pair[0] == pair[1])
                    .map(|pair| builder.items.weights[pair[0] as usize])
                    .sum();
                if repeated > 0 {
                    let mut items: Vec<u32> = builder.items.of(&candidate).collect();
                    candidate.listed = Some(builder.list(&mut items));
                    cost -= repeated;
                }
                candidate.cost = cost;

                #[cfg(test)]
                assert_eq!(
                    candidate.cost,
                    tree_cost(graph, &candidate.build_tree(graph)?.root),
                    "the cost from the items must match the cost of the tree"
                );

                alternatives.push(candidate);
            }
            // Cheapest on its own first, it wins ties when picking.
            alternatives.sort_by_key(|candidate| candidate.cost);
            groups.push(alternatives);
        }
    }

    // Greedy goes in this order: the sets with the fewest choices first.
    groups.sort_by_key(|alternatives| alternatives.len());

    Ok((groups, builder.items))
}

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

    let (groups, items) = prepare_alternatives(graph, operation)?;
    if groups.is_empty() {
        return Err(QueryPlanError::EmptyPlan);
    }

    let mut plan = Plan::new(&items, &groups);
    plan.pick_greedily(cancellation_token)?;
    let greedy_cost = plan.cost();
    let (mut lower_bound, sets_using) = lower_bound(&items, &groups);
    // The cheapest plan costs a whole number, at least the bound.
    let is_proven = |cost: u64, bound: f64| cost as f64 <= (bound - 1e-6).ceil();
    let mut proven = is_proven(greedy_cost, lower_bound);
    if !proven {
        let (picks, bound) = relax(
            &items,
            &groups,
            &sets_using,
            greedy_cost,
            cancellation_token,
        )?;
        drop(sets_using);
        if let Some(picks) = picks {
            plan.reset(picks);
        }
        lower_bound = lower_bound.max(bound);
        proven = is_proven(plan.cost(), lower_bound);
    }
    let relaxed_cost = plan.cost();
    if !proven {
        plan.improve(cancellation_token)?;
        proven = is_proven(plan.cost(), lower_bound);
    }
    trace!(
        greedy_cost,
        relaxed_cost,
        lower_bound,
        proven,
        cost = plan.cost(),
        "picked candidates"
    );

    #[cfg(test)]
    let cost = plan.cost();
    let choice = plan.choice;
    drop(items);
    let tree = merge_picks(graph, groups, &choice)?;

    #[cfg(test)]
    assert_eq!(
        cost,
        tree_cost(graph, &tree.root),
        "the cost from the items must match the cost of the merged tree"
    );

    Ok(tree)
}

/// The picked candidates as one tree, merged in the order of the sets. Takes the sets by value,
/// so each one's paths are freed once its pick is in the tree.
fn merge_picks(
    graph: &Graph<'_>,
    groups: Vec<Alternatives>,
    choice: &[usize],
) -> Result<QueryTree, QueryPlanError> {
    let mut plan: Option<QueryTree> = None;
    for (alternatives, &picked) in groups.into_iter().zip(choice) {
        let tree = alternatives[picked].build_tree(graph)?;
        match plan.as_mut() {
            Some(plan) => Rc::make_mut(&mut plan.root).merge_nodes(&tree.root),
            None => plan = Some(tree),
        }
    }
    plan.ok_or(QueryPlanError::EmptyPlan)
}

/// The picked candidate of every set, and how many of them use each item.
struct Plan<'a> {
    /// `items.weights`, kept at hand for the hot loops.
    weights: &'a [u64],
    items: &'a Items,
    groups: &'a [Alternatives],
    choice: Vec<usize>,
    uses: Vec<u32>,
    /// The weight of the items in use. The plan's root isn't an item, see `cost`.
    total: u64,
    /// Marks a candidate's items, to tell which items two candidates share. Only the local search
    /// needs it.
    marks: Vec<u32>,
    mark: u32,
}

impl<'a> Plan<'a> {
    fn new(items: &'a Items, groups: &'a [Alternatives]) -> Self {
        Self {
            weights: &items.weights,
            items,
            groups,
            choice: vec![0; groups.len()],
            uses: vec![0; items.weights.len()],
            total: 0,
            marks: Vec::new(),
            mark: 0,
        }
    }

    fn of(&self, candidate: &'a Candidate) -> CandidateItems<'a> {
        self.items.of(candidate)
    }

    fn cost(&self) -> u64 {
        FIELD_COST + self.total
    }

    fn items(&self, group: usize) -> CandidateItems<'a> {
        self.of(&self.groups[group][self.choice[group]])
    }

    /// What `items` would add to the plan.
    fn added(&self, items: CandidateItems<'_>) -> u64 {
        items
            .filter(|&item| self.uses[item as usize] == 0)
            .map(|item| self.weights[item as usize])
            .sum()
    }

    fn add(&mut self, items: CandidateItems<'_>) {
        for item in items {
            if self.uses[item as usize] == 0 {
                self.total += self.weights[item as usize];
            }
            self.uses[item as usize] += 1;
        }
    }

    fn remove(&mut self, items: CandidateItems<'_>) {
        for item in items {
            self.uses[item as usize] -= 1;
            if self.uses[item as usize] == 0 {
                self.total -= self.weights[item as usize];
            }
        }
    }

    /// Starts over from these picks.
    fn reset(&mut self, choice: Vec<usize>) {
        self.uses.fill(0);
        self.total = 0;
        self.choice = choice;
        for group in 0..self.groups.len() {
            self.add(self.items(group));
        }
    }

    fn set(&mut self, group: usize, candidate: usize) {
        self.remove(self.items(group));
        self.choice[group] = candidate;
        self.add(self.items(group));
    }

    /// For each set, the candidate that adds the least cost to the plan so far.
    fn pick_greedily(
        &mut self,
        cancellation_token: &CancellationToken,
    ) -> Result<(), QueryPlanError> {
        let groups = self.groups;
        for (group, alternatives) in groups.iter().enumerate() {
            cancellation_token.bail_if_cancelled()?;

            let mut best = 0;
            let mut best_added_cost = u64::MAX;
            for (index, candidate) in alternatives.iter().enumerate() {
                let added_cost = self.added(self.of(candidate));
                if added_cost < best_added_cost {
                    best_added_cost = added_cost;
                    best = index;
                }
            }
            self.choice[group] = best;
            self.add(self.of(&alternatives[best]));
        }
        Ok(())
    }

    /// Switches sets to other candidates while that makes the plan cheaper, until neither move
    /// finds anything or the work budget runs out.
    fn improve(&mut self, cancellation_token: &CancellationToken) -> Result<(), QueryPlanError> {
        self.marks = vec![0; self.weights.len()];

        // The sets that could use each crossing, for moving them all off it.
        let mut candidate_items = 0;
        let mut users: HashMap<u32, Vec<u32>> = HashMap::new();
        for (group, alternatives) in self.groups.iter().enumerate() {
            for candidate in alternatives {
                for item in self.of(candidate) {
                    candidate_items += 1;
                    if self.weights[item as usize] >= CROSS_SUBGRAPH_COST {
                        let sets = users.entry(item).or_default();
                        if sets.last() != Some(&(group as u32)) {
                            sets.push(group as u32);
                        }
                    }
                }
            }
        }
        let mut crossings: Vec<u32> = users.keys().copied().collect();
        crossings.sort_unstable();
        let mut budget = candidate_items * SEARCH_WORK_PER_CANDIDATE_ITEM;

        loop {
            let switched = self.switch_one(&mut budget, cancellation_token)?;
            let dropped =
                self.drop_crossings(&crossings, &users, &mut budget, cancellation_token)?;
            if (!switched && !dropped) || budget == 0 {
                return Ok(());
            }
        }
    }

    /// Gives each set the candidate that makes the plan cheapest, the others staying as they are.
    fn switch_one(
        &mut self,
        budget: &mut usize,
        cancellation_token: &CancellationToken,
    ) -> Result<bool, QueryPlanError> {
        let mut improved = false;
        let groups = self.groups;
        for (group, alternatives) in groups.iter().enumerate() {
            if alternatives.len() < 2 {
                continue;
            }
            cancellation_token.bail_if_cancelled()?;

            let mut best = self.choice[group];
            let mut best_change: i64 = 0;
            for (index, candidate) in alternatives.iter().enumerate() {
                if index == self.choice[group] {
                    continue;
                }
                let (change, work) = self.switch_change(group, candidate);
                *budget = budget.saturating_sub(work);
                if change < best_change {
                    best_change = change;
                    best = index;
                }
            }
            if best != self.choice[group] {
                self.set(group, best);
                improved = true;
            }
            if *budget == 0 {
                break;
            }
        }
        Ok(improved)
    }

    /// How the plan's cost changes when `group` uses `next` instead of its current candidate,
    /// and how many items that looked at.
    fn switch_change(&mut self, group: usize, next: &'a Candidate) -> (i64, usize) {
        self.mark += 1;
        let mut work = 0;
        let mut added = 0;
        for item in self.of(next) {
            work += 1;
            self.marks[item as usize] = self.mark;
            if self.uses[item as usize] == 0 {
                added += self.weights[item as usize];
            }
        }
        // The current candidate's items only it uses go away, unless `next` has them too.
        let mut freed = 0;
        for item in self.items(group) {
            work += 1;
            if self.uses[item as usize] == 1 && self.marks[item as usize] != self.mark {
                freed += self.weights[item as usize];
            }
        }
        (added as i64 - freed as i64, work)
    }

    /// For each crossing in use, moves every set using it to the best candidate without it, and
    /// keeps that when the plan is cheaper.
    fn drop_crossings(
        &mut self,
        crossings: &[u32],
        users: &HashMap<u32, Vec<u32>>,
        budget: &mut usize,
        cancellation_token: &CancellationToken,
    ) -> Result<bool, QueryPlanError> {
        let mut improved = false;
        for &crossing in crossings {
            if *budget == 0 {
                break;
            }
            if self.uses[crossing as usize] == 0 {
                continue;
            }
            cancellation_token.bail_if_cancelled()?;

            // Every item it looks at goes on the budget, even when the crossing can't be dropped.
            let items = self.items;
            let without = |candidate: &'a Candidate, work: &mut usize| {
                items.of(candidate).all(|item| {
                    *work += 1;
                    item != crossing
                })
            };
            let mut work = 0;
            let sets: Vec<usize> = users[&crossing]
                .iter()
                .map(|&group| group as usize)
                .filter(|&group| !without(&self.groups[group][self.choice[group]], &mut work))
                .collect();
            let movable = sets.iter().all(|&group| {
                self.groups[group]
                    .iter()
                    .any(|candidate| without(candidate, &mut work))
            });
            *budget = budget.saturating_sub(work);
            if !movable {
                continue;
            }

            let before = self.total;
            let previous: Vec<usize> = sets.iter().map(|&group| self.choice[group]).collect();
            for &group in &sets {
                self.remove(self.items(group));
            }
            for &group in &sets {
                let mut best = usize::MAX;
                let mut best_added_cost = u64::MAX;
                let groups = self.groups;
                let mut work = 0;
                for (index, candidate) in groups[group].iter().enumerate() {
                    if !without(candidate, &mut work) {
                        continue;
                    }
                    let added_cost = self.added(self.of(candidate));
                    if added_cost < best_added_cost {
                        best_added_cost = added_cost;
                        best = index;
                    }
                }
                *budget = budget.saturating_sub(work);
                self.choice[group] = best;
                self.add(self.items(group));
            }

            if self.total < before {
                improved = true;
            } else {
                for (&group, &candidate) in sets.iter().zip(&previous) {
                    self.set(group, candidate);
                }
            }
        }
        Ok(improved)
    }
}

/// No plan costs less. Items every candidate of a set has are paid for sure. Any other item is
/// paid once by the sets using it, so each of them pays at least its weight divided by the number
/// of sets that could use it, and each set pays at least its cheapest candidate's share.
/// Also returns how many sets could use each item.
fn lower_bound(items: &Items, groups: &[Alternatives]) -> (f64, Vec<u32>) {
    let weights = &items.weights;
    let mut sets_using = vec![0u32; weights.len()];
    let mut in_candidates = vec![0u32; weights.len()];
    let mut seen_in = vec![u32::MAX; weights.len()];
    let mut certain = vec![false; weights.len()];

    for (group, alternatives) in groups.iter().enumerate() {
        let group = group as u32;
        for candidate in alternatives {
            for item in items.of(candidate) {
                let item = item as usize;
                if seen_in[item] != group {
                    seen_in[item] = group;
                    in_candidates[item] = 0;
                    sets_using[item] += 1;
                }
                in_candidates[item] += 1;
            }
        }
        for item in items.of(&alternatives[0]) {
            if in_candidates[item as usize] as usize == alternatives.len() {
                certain[item as usize] = true;
            }
        }
    }

    let certain_cost: u64 = (0..weights.len())
        .filter(|&item| certain[item])
        .map(|item| weights[item])
        .sum();
    let shares: f64 = groups
        .iter()
        .map(|alternatives| {
            alternatives
                .iter()
                .map(|candidate| {
                    items
                        .of(candidate)
                        .filter(|&item| !certain[item as usize])
                        .map(|item| {
                            weights[item as usize] as f64 / sets_using[item as usize] as f64
                        })
                        .sum::<f64>()
                })
                .fold(f64::MAX, f64::min)
        })
        .sum();

    ((FIELD_COST + certain_cost) as f64 + shares, sets_using)
}

/// At most this many steps of the relaxation, and it stops early after this many in a row without
/// a cheaper plan.
const RELAXATION_STEPS: usize = 30;
const RELAXATION_PATIENCE: usize = 5;

/// Lagrangian relaxation, the usual way to get both cheap plans and lower bounds for set cover.
///
/// The constraint "an item is paid when a picked candidate has it" is dropped, and every item
/// gets a price `price[i]` instead, shared by the sets that could use it. With prices, each set
/// just takes its candidate with the lowest price, and that's a real plan: often a much cheaper
/// one than greedy's, because it sees from the start which items many sets will share. The
/// prices start at weight divided by sets, then move by subgradient steps: up for items that
/// many picks use but aren't paid for, down for items that are paid for but barely used.
/// Every step also gives a lower bound.
///
/// Returns the cheapest picks found, when cheaper than `upper`, and the best bound.
fn relax(
    items: &Items,
    groups: &[Alternatives],
    sets_using: &[u32],
    upper: u64,
    cancellation_token: &CancellationToken,
) -> Result<(Option<Vec<usize>>, f64), QueryPlanError> {
    let weights = &items.weights;
    let mut price: Vec<f64> = weights
        .iter()
        .zip(sets_using)
        .map(|(&weight, &sets)| match sets {
            0 => 0.0,
            sets => weight as f64 / sets as f64,
        })
        .collect();
    let mut picks = vec![0; groups.len()];
    let mut picked_by = vec![0u32; weights.len()];
    let mut best_picks = None;
    let mut best_cost = upper;
    let mut best_bound = f64::MIN;
    let mut step_size = 2.0;
    let mut no_better_bound = 0;
    let mut no_better_plan = 0;

    for _ in 0..RELAXATION_STEPS {
        cancellation_token.bail_if_cancelled()?;

        // Each set takes its cheapest candidate at these prices. The first one wins a tie, and
        // candidates are sorted by their own cost.
        let mut bound = FIELD_COST as f64;
        picked_by.fill(0);
        for (group, alternatives) in groups.iter().enumerate() {
            let mut best = 0;
            let mut best_price = f64::MAX;
            for (index, candidate) in alternatives.iter().enumerate() {
                let total: f64 = items.of(candidate).map(|item| price[item as usize]).sum();
                if total < best_price {
                    best_price = total;
                    best = index;
                }
            }
            picks[group] = best;
            bound += best_price;
            for item in items.of(&alternatives[best]) {
                picked_by[item as usize] += 1;
            }
        }

        // An item is worth paying for when the sets that could use it pay more than it weighs.
        let mut cost = FIELD_COST;
        for item in 0..weights.len() {
            let unpaid = weights[item] as f64 - price[item] * sets_using[item] as f64;
            if unpaid < 0.0 {
                bound += unpaid;
            }
            if picked_by[item] > 0 {
                cost += weights[item];
            }
        }

        if cost < best_cost {
            best_cost = cost;
            best_picks = Some(picks.clone());
            no_better_plan = 0;
        } else {
            no_better_plan += 1;
        }
        if bound > best_bound + 1e-9 {
            best_bound = bound;
            no_better_bound = 0;
        } else {
            no_better_bound += 1;
            if no_better_bound >= 3 {
                step_size /= 2.0;
                no_better_bound = 0;
            }
        }
        let proven = best_cost as f64 <= (best_bound - 1e-6).ceil();
        if proven || no_better_plan >= RELAXATION_PATIENCE {
            break;
        }

        // The subgradient: how many picks use an item, minus how many sets pay for it.
        let gradients: Vec<f64> = (0..weights.len())
            .map(|item| {
                let sets = sets_using[item] as f64;
                let paid = (weights[item] as f64) < price[item] * sets;
                picked_by[item] as f64 - if paid { sets } else { 0.0 }
            })
            .collect();
        let norm: f64 = gradients.iter().map(|gradient| gradient * gradient).sum();
        if norm == 0.0 {
            break;
        }
        let step = step_size * (best_cost as f64 - bound) / norm;
        for (price, gradient) in price.iter_mut().zip(gradients) {
            *price = (*price + step * gradient).max(0.0);
        }
    }

    Ok((best_picks, best_bound))
}

/// The cost of a tree, worked out on the tree itself: a field per node, a crossing per fetch,
/// keyed like `FetchKey`, and a crossing per node that resolves context. Test builds check the
/// items against it.
#[cfg(test)]
fn tree_cost(graph: &Graph<'_>, root: &QueryTreeNode) -> u64 {
    use ahash::HashSet;

    #[derive(PartialEq, Eq, Hash)]
    struct Fetch {
        entity_call: bool,
        subgraph: Option<u16>,
        path: String,
        mutation_field_position: MutationFieldPosition,
    }

    fn walk(
        graph: &Graph<'_>,
        node: &QueryTreeNode,
        path: &str,
        parent_path: &str,
        mutation_field_position: MutationFieldPosition,
        nodes: &mut u64,
        fetches: &mut HashSet<Fetch>,
    ) {
        *nodes += 1;
        let mutation_field_position = node.mutation_field_position.or(mutation_field_position);
        let subgraph = graph.node(node.node_index).unwrap().subgraph_index();
        let edge = node.edge_from_parent.map(|edge| graph.edge(edge).unwrap());
        if matches!(edge, Some(Edge::SubgraphEntrypoint { .. })) {
            fetches.insert(Fetch {
                entity_call: false,
                subgraph,
                path: String::new(),
                mutation_field_position,
            });
        }
        let object_path = match edge {
            Some(Edge::FieldMove(_)) => parent_path,
            _ => path,
        };
        for requirement in &node.requirements {
            fetches.insert(Fetch {
                entity_call: true,
                subgraph,
                path: object_path.to_string(),
                mutation_field_position,
            });
            walk(
                graph,
                requirement,
                object_path,
                object_path,
                mutation_field_position,
                nodes,
                fetches,
            );
        }
        for context in node.context_requirements() {
            walk(
                graph,
                context,
                path,
                path,
                mutation_field_position,
                nodes,
                fetches,
            );
        }
        for child in &node.children {
            let child_path = match child.edge_from_parent.map(|edge| graph.edge(edge).unwrap()) {
                Some(Edge::FieldMove(field)) => {
                    let response_key = child
                        .selection_attributes
                        .as_ref()
                        .and_then(|attributes| attributes.alias.as_deref())
                        .unwrap_or(field.name);
                    format!("{path}.{response_key}")
                }
                _ => path.to_string(),
            };
            walk(
                graph,
                child,
                &child_path,
                path,
                mutation_field_position,
                nodes,
                fetches,
            );
        }
    }

    fn resolving_context(node: &QueryTreeNode) -> u64 {
        u64::from(!node.resolved_context_requirements().is_empty())
            + node
                .children
                .iter()
                .chain(&node.requirements)
                .chain(node.context_requirements())
                .map(|node| resolving_context(node))
                .sum::<u64>()
    }

    let mut nodes = 0;
    let mut fetches = HashSet::default();
    walk(graph, root, "", "", None, &mut nodes, &mut fetches);
    nodes * FIELD_COST + (fetches.len() as u64 + resolving_context(root)) * CROSS_SUBGRAPH_COST
}
