//! Picks one path per leaf of the query, so the plan they make together is the best one.
//!
//! A plan is better with fewer waves of fetches that run one after another, then with fewer
//! fetches, then with a smaller tree. Fetches are counted the way the fetch graph makes
//! them (`FetchKey`): a subgraph's root fetch, or the entity calls to a subgraph after as many
//! other entity calls, however many paths need them.
//!
//! Trying every combination is exponentially slow, the number of plans is the product of the
//! number of choices for each leaf. So it's greedy: leaves with one way to get them go in a
//! `BasePlan` first, then a leaf at a time, fewest choices first, each takes the candidate that
//! adds the least to the plan so far, the cheapest on its own on a tie.
//!
//! A `Candidate` is costed and merged from its path, its `QueryTree` is built only when needed.

use ahash::{HashMap, HashMapExt, HashSet};
use petgraph::graph::NodeIndex;
use std::{cell::OnceCell, cmp::Reverse, collections::BTreeMap, rc::Rc};

use crate::query_planner::{
    graph::{edge::Edge, error::GraphError, Graph},
    planner::{
        error::QueryPlanError,
        tree::{
            query_tree::QueryTree,
            query_tree_node::{merge_query_tree_node_list, MutationFieldPosition, QueryTreeNode},
        },
        walker::{
            path::{same_attributes, OperationPath, PathSegment},
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

    let base = BasePlan::merge(graph, leaves.singles, cancellation_token)?;
    let groups = leaves
        .groups
        .into_iter()
        .map(|paths| Candidate::alternatives(graph, paths))
        .collect::<Result<Vec<_>, _>>()?;

    // the fetches each leaf could use, and how many of the leaves still to pick could use each
    let usable: Vec<Vec<FetchKey>> = groups
        .iter()
        .map(|alternatives| {
            let mut keys: Vec<FetchKey> = alternatives
                .iter()
                .flat_map(|candidate| candidate.fetches.iter().copied())
                .collect();
            keys.sort_unstable();
            keys.dedup();
            keys
        })
        .collect();
    let mut users: HashMap<FetchKey, u32> = HashMap::new();
    for key in usable.iter().flatten() {
        *users.entry(*key).or_default() += 1;
    }

    let base_keys = base.fetches.keys.clone();
    let mut plan = base.tree.clone();
    let mut fetches = base.fetches;
    let mut picks = Vec::with_capacity(groups.len());
    for (position, (alternatives, usable)) in groups.iter().zip(&usable).enumerate() {
        cancellation_token.bail_if_cancelled()?;
        for key in usable {
            *users.get_mut(key).expect("counted above") -= 1;
        }
        let pick = cheapest_to_add(
            graph,
            alternatives,
            plan.as_ref(),
            &fetches,
            &users,
            &groups[position + 1..],
        )?;
        fetches.add(&alternatives[pick]);
        add(graph, &mut plan, &alternatives[pick])?;
        picks.push(pick);
    }

    if drop_fetches(&groups, &mut picks, &base_keys) {
        plan = base.tree;
        for (alternatives, pick) in groups.iter().zip(picks) {
            add(graph, &mut plan, &alternatives[pick])?;
        }
    }

    plan.ok_or(QueryPlanError::EmptyPlan)
}

fn add(
    graph: &Graph<'_>,
    plan: &mut Option<QueryTree>,
    candidate: &Candidate<'_>,
) -> Result<(), GraphError> {
    match plan.as_mut() {
        Some(tree) => candidate.merge_into(graph, Rc::make_mut(&mut tree.root)),
        None => {
            *plan = Some(candidate.tree(graph)?);
            Ok(())
        }
    }
}

/// Takes out the fetches greedy opened that the plan can do without: one goes when every leaf that
/// uses it has another candidate whose fetches the plan makes anyway. Greedy goes a leaf at a time,
/// so it can open a fetch for a few leaves that other fetches it opens later cover too. Waves can't
/// go up, they come from the fetches. Returns whether a pick changed.
fn drop_fetches(
    groups: &[Alternatives<'_>],
    picks: &mut [usize],
    base: &HashSet<FetchKey>,
) -> bool {
    // how many picks use each fetch
    let mut uses: HashMap<FetchKey, u32> = HashMap::new();
    for (alternatives, &pick) in groups.iter().zip(picks.iter()) {
        for key in &alternatives[pick].fetches {
            *uses.entry(*key).or_default() += 1;
        }
    }
    let mut changed = false;
    loop {
        // the least used first, sorted so plans don't depend on hashing
        let mut keys: Vec<(u32, FetchKey)> = uses
            .iter()
            .filter(|(key, uses)| **uses > 0 && !base.contains(*key))
            .map(|(key, uses)| (*uses, *key))
            .collect();
        keys.sort_unstable();
        let mut dropped = false;
        for (_, key) in keys {
            let made = |other: &FetchKey| {
                *other != key && (base.contains(other) || uses.get(other).is_some_and(|n| *n > 0))
            };
            let mut moves = Vec::new();
            let all_move =
                groups
                    .iter()
                    .zip(picks.iter())
                    .enumerate()
                    .all(|(leaf, (alternatives, &pick))| {
                        if !alternatives[pick].fetches.contains(&key) {
                            return true;
                        }
                        let other = alternatives
                            .iter()
                            .position(|candidate| candidate.fetches.iter().all(made));
                        other.inspect(|&other| moves.push((leaf, other))).is_some()
                    });
            if !all_move || moves.is_empty() {
                continue;
            }
            for (leaf, other) in moves {
                for key in &groups[leaf][picks[leaf]].fetches {
                    *uses.get_mut(key).expect("counted") -= 1;
                }
                for key in &groups[leaf][other].fetches {
                    *uses.entry(*key).or_default() += 1;
                }
                picks[leaf] = other;
            }
            dropped = true;
            changed = true;
            break;
        }
        if !dropped {
            return changed;
        }
    }
}

/// The candidate that adds the fewest waves, then fetches, to `plan`. Between those, the one whose
/// new fetches more of the leaves still to pick could use too, so they don't each open their own.
/// Then the one that adds the least to the tree, and the first one, they're cheapest on their own
/// first. Each tie-break is only worked out for the candidates still tied.
fn cheapest_to_add(
    graph: &Graph<'_>,
    alternatives: &Alternatives<'_>,
    plan: Option<&QueryTree>,
    fetches: &Fetches,
    users: &HashMap<FetchKey, u32>,
    remaining: &[Alternatives<'_>],
) -> Result<usize, GraphError> {
    let tied = most_by(0..alternatives.len(), |index| {
        let (waves, added) = fetches.added_by(&alternatives[index]);
        Reverse((waves, added))
    });
    if let [index] = tied[..] {
        return Ok(index);
    }
    let tied = most_by(tied.into_iter(), |index| {
        alternatives[index]
            .fetches
            .iter()
            .filter(|key| !fetches.keys.contains(*key))
            .map(|key| users.get(key).copied().unwrap_or(0))
            .sum::<u32>()
    });
    if let [index] = tied[..] {
        return Ok(index);
    }
    // Then the one more leaves still to pick need nothing else for: their whole path is in the
    // fetches the plan makes with it. Two root fetches every leaf could use tie above, but the one
    // that has a field itself, like one with `@provides`, saves the other leaves a hop.
    let tied = most_by(tied.into_iter(), |index| {
        let added: Vec<&FetchKey> = alternatives[index]
            .fetches
            .iter()
            .filter(|key| !fetches.keys.contains(*key))
            .collect();
        remaining
            .iter()
            .filter(|leaf| {
                leaf.iter().any(|candidate| {
                    candidate
                        .fetches
                        .iter()
                        .all(|key| fetches.keys.contains(key) || added.contains(&key))
                })
            })
            .count()
    });
    if let [index] = tied[..] {
        return Ok(index);
    }
    let mut cheapest = None;
    for index in tied {
        let candidate = &alternatives[index];
        let added_cost = match plan {
            Some(tree) => candidate.added_cost(graph, &tree.root)?,
            None => candidate.cost,
        };
        if cheapest.is_none_or(|(cost, _)| added_cost < cost) {
            cheapest = Some((added_cost, index));
        }
    }
    let (_, index) = cheapest.expect("a leaf has a way to get it");
    Ok(index)
}

/// The indexes with the highest `rank`, in order.
fn most_by<R: Ord>(indexes: impl Iterator<Item = usize>, rank: impl Fn(usize) -> R) -> Vec<usize> {
    let mut best = None;
    let mut tied = Vec::new();
    for index in indexes {
        let rank = rank(index);
        if best.as_ref().is_none_or(|best| rank > *best) {
            best = Some(rank);
            tied.clear();
            tied.push(index);
        } else if best.as_ref() == Some(&rank) {
            tied.push(index);
        }
    }
    tied
}

/// Every plan the leaves' paths make, one per pick of a path for each leaf, handed to `each`.
/// For the plan oracle test, to check what the passes above pick against all of them.
/// `None` when there are more than `limit` of them, otherwise how many there were.
#[cfg(test)]
pub(crate) fn every_combination(
    graph: &Graph<'_>,
    operation: ResolvedOperation,
    limit: usize,
    mut each: impl FnMut(QueryTree),
) -> Result<Option<usize>, QueryPlanError> {
    let leaves = Leaves::of(operation);
    let base = BasePlan::merge(graph, leaves.singles, &CancellationToken::new())?;
    let groups = leaves
        .groups
        .into_iter()
        .map(|paths| Candidate::alternatives(graph, paths))
        .collect::<Result<Vec<_>, _>>()?;
    let Some(count) = groups.iter().try_fold(1usize, |count, alternatives| {
        count
            .checked_mul(alternatives.len())
            .filter(|count| *count <= limit)
    }) else {
        return Ok(None);
    };

    // counts up like an odometer, one digit per leaf
    let mut picks = vec![0; groups.len()];
    loop {
        let mut plan = base.tree.clone();
        let merged = groups.iter().zip(&picks).all(|(alternatives, &pick)| {
            let candidate = &alternatives[pick];
            match plan.as_mut() {
                Some(tree) => candidate
                    .merge_into(graph, Rc::make_mut(&mut tree.root))
                    .is_ok(),
                None => candidate.tree(graph).map(|tree| plan = Some(tree)).is_ok(),
            }
        });
        if let (true, Some(tree)) = (merged, plan) {
            each(tree);
        }
        let mut digit = 0;
        loop {
            if digit == picks.len() {
                return Ok(Some(count));
            }
            picks[digit] += 1;
            if picks[digit] < groups[digit].len() {
                break;
            }
            picks[digit] = 0;
            digit += 1;
        }
    }
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
    fetches: Fetches,
}

impl BasePlan {
    fn merge(
        graph: &Graph<'_>,
        singles: Vec<PathAndPosition<'_>>,
        cancellation_token: &CancellationToken,
    ) -> Result<Self, QueryPlanError> {
        let mut base = BasePlan {
            tree: None,
            fetches: Fetches::default(),
        };

        for (path, mutation_field_position) in singles {
            cancellation_token.bail_if_cancelled()?;
            let candidate = Candidate::new(graph, path, mutation_field_position)?;
            base.fetches.add(&candidate);
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
    /// The fetches of its tree, see `FetchKey`.
    fetches: Vec<FetchKey>,
    /// Its nodes that resolve context, a fetch each.
    crossings: u64,
    tree: OnceCell<QueryTree>,
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
        let mut walk = FetchWalk::new(graph);
        let cost = if uses_context {
            let built = QueryTree::from_path(graph, &path, mutation_field_position)?;
            let cost = Costs(graph).tree(&built.root);
            walk.node(&built.root, 0, 0, None);
            let _ = tree.set(built);
            cost
        } else {
            // one walk over the path for both
            let segments = path.get_segments();
            walk.path(&segments, mutation_field_position)?;
            FIELD_COST + Costs(graph).of_segments(&segments)?
        };
        // what's worked out from the path has to match what its tree gives
        #[cfg(test)]
        if !uses_context {
            let built = QueryTree::from_path(graph, &path, mutation_field_position)?;
            let mut from_tree = FetchWalk::new(graph);
            from_tree.node(&built.root, 0, 0, None);
            let mut from_path = FetchWalk::new(graph);
            from_path.path(&path.get_segments(), mutation_field_position)?;
            assert_eq!(cost, Costs(graph).tree(&built.root), "path cost");
            let sorted = |mut fetches: Vec<FetchKey>| {
                fetches.sort_unstable();
                fetches
            };
            assert_eq!(
                sorted(from_path.fetches),
                sorted(from_tree.fetches),
                "path fetches"
            );
            assert_eq!(from_path.crossings, from_tree.crossings, "path crossings");
        }
        Ok(Self {
            path,
            mutation_field_position,
            uses_context,
            cost,
            fetches: walk.fetches,
            crossings: walk.crossings,
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

/// A fetch the way the fetch graph makes it: a subgraph's root fetch, or the entity calls to a
/// subgraph after as many other entity calls (`true`, depth). The fetch graph batches calls to a
/// subgraph across response paths, so it's per depth, not per object.
type FetchKey = (bool, Option<u16>, u32, MutationFieldPosition);

/// Which wave a fetch runs in: root fetches first, entity calls after as many waves as entity
/// calls before them.
fn wave((entity_call, _, depth, _): &FetchKey) -> u32 {
    match entity_call {
        true => depth + 2,
        false => 1,
    }
}

/// The fetches a plan makes so far, with the waves of each mutation field (mutation fields run
/// one after another, the fields of a query all at once).
#[derive(Default)]
struct Fetches {
    keys: HashSet<FetchKey>,
    crossings: u64,
    waves: BTreeMap<MutationFieldPosition, u32>,
}

impl Fetches {
    fn add(&mut self, candidate: &Candidate<'_>) {
        for key in &candidate.fetches {
            let waves = self.waves.entry(key.3).or_default();
            *waves = (*waves).max(wave(key));
            self.keys.insert(*key);
        }
        self.crossings += candidate.crossings;
    }

    /// The waves and fetches adding `candidate` adds to the plan.
    // ponytail: depth stands in for the batching the fetch graph does, it's only exact when
    // calls at the same depth don't depend on each other. Checked against real plans by the
    // plan oracle test.
    /// A path's fetches are all at its mutation field's position.
    fn added_by(&self, candidate: &Candidate<'_>) -> (u32, u64) {
        let fetches = candidate.crossings
            + candidate
                .fetches
                .iter()
                .filter(|key| !self.keys.contains(*key))
                .count() as u64;
        let waves = candidate.fetches.first().map_or(0, |first| {
            let most = candidate.fetches.iter().map(wave).max().unwrap_or(0);
            most.saturating_sub(self.waves.get(&first.3).copied().unwrap_or(0))
        });
        (waves, fetches)
    }
}

/// Walks a tree for its fetches, with how many entity calls lead to each node.
struct FetchWalk<'a> {
    graph: &'a Graph<'a>,
    /// The nodes that resolve context, a fetch each
    crossings: u64,
    /// A path makes a few, so a list, without repeats
    fetches: Vec<FetchKey>,
}

impl<'a> FetchWalk<'a> {
    fn new(graph: &'a Graph<'a>) -> Self {
        Self {
            graph,
            crossings: 0,
            fetches: Vec::new(),
        }
    }

    fn insert(&mut self, fetch: FetchKey) {
        if !self.fetches.contains(&fetch) {
            self.fetches.push(fetch);
        }
    }

    /// Same as `node` on the tree `QueryTree::from_path` makes of `segments`, a node per segment
    /// under a root, without building it. Not for `@fromContext` paths.
    fn path(
        &mut self,
        segments: &[&PathSegment],
        position: MutationFieldPosition,
    ) -> Result<(), GraphError> {
        let (mut depth, mut object_depth) = (0, 0);
        for (index, segment) in segments.iter().enumerate() {
            let edge = self.graph.edge(segment.edge_index)?;
            let subgraph = self
                .graph
                .node(self.graph.get_edge_tail(&segment.edge_index)?)?
                .subgraph_index();
            if matches!(edge, Edge::SubgraphEntrypoint { .. }) {
                self.insert((false, subgraph, 0, position));
            }
            let Some(requirement) = &segment.requirement_tree else {
                object_depth = depth;
                continue;
            };
            let is_requires = matches!(edge, Edge::FieldMove(_));
            let requirement_depth = if is_requires { object_depth } else { depth };
            let only_requires = match segments.get(index + 1) {
                Some(next) => {
                    next.requirement_tree.is_some()
                        && matches!(self.graph.edge(next.edge_index)?, Edge::FieldMove(_))
                }
                None => false,
            };
            // the call waits for the calls that get what it needs
            let call_depth = self
                .node(requirement, requirement_depth, requirement_depth, position)
                .map_or(depth, |deepest| depth.max(deepest + 1));
            if is_requires || !only_requires {
                self.insert((true, subgraph, call_depth, position));
            }
            object_depth = if is_requires {
                call_depth + 1
            } else {
                call_depth
            };
            depth = call_depth + 1;
        }
        Ok(())
    }

    /// `object_depth` is where the calls for a @requires on this node's object go: next to the
    /// entity call that brought the object in, as the requirement is read from where it came from,
    /// or after the fetch it came in with. Returns the depth of the deepest entity call in it.
    fn node(
        &mut self,
        node: &QueryTreeNode,
        depth: u32,
        object_depth: u32,
        position: MutationFieldPosition,
    ) -> Option<u32> {
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
            self.insert((false, subgraph, 0, position));
        }
        let is_field = |node: &QueryTreeNode| {
            node.edge_from_parent
                .is_some_and(|edge| matches!(self.graph.edge(edge), Ok(Edge::FieldMove(_))))
        };
        let is_requires = is_field(node);
        let requirement_depth = if is_requires { object_depth } else { depth };
        // An entity call that only brings in fields with a @requires has no fetch of its own, the
        // fetch graph asks for the object in the calls for those fields.
        let only_requires = !node.children.is_empty()
            && node
                .children
                .iter()
                .all(|child| !child.requirements.is_empty() && is_field(child));
        // the call waits for the calls that get what it needs
        let mut deepest = None;
        let mut call_depth = depth;
        for requirement in &node.requirements {
            let needs = self.node(requirement, requirement_depth, requirement_depth, position);
            if let Some(needs) = needs {
                call_depth = call_depth.max(needs + 1);
            }
            deepest = deepest.max(needs);
        }
        if !node.requirements.is_empty() && (is_requires || !only_requires) {
            self.insert((true, subgraph, call_depth, position));
            deepest = deepest.max(Some(call_depth));
        }
        for context in node.context_requirements() {
            deepest = deepest.max(self.node(context, depth, object_depth, position));
        }
        let child_depth = match node.requirements.is_empty() {
            true => depth,
            false => call_depth + 1,
        };
        // an entity call's fields are on the object it brought in, the rest are after this fetch
        let entered = !node.requirements.is_empty() && !is_requires;
        let child_object_depth = if entered { call_depth } else { child_depth };
        for child in &node.children {
            deepest = deepest.max(self.node(child, child_depth, child_object_depth, position));
        }
        deepest
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
            && child.mutation_field_position == position
            && child.condition == segment.condition
            && child.context_coordinate().is_none()
            && same_attributes(&child.selection_attributes, &segment.selection_attributes)
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
