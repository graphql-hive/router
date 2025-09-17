use std::{cell::OnceCell, collections::HashMap, sync::Arc};

use lazy_init::LazyTransform;
use xxhash_rust::xxh3::Xxh3;

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
};

type PathAndPosition = (OperationPath, MutationFieldPosition);
type QueryTreeResult = Result<QueryTree, GraphError>;
type LazyQueryTree = LazyTransform<PathAndPosition, QueryTreeResult>;
type Group = Vec<Candidate>;

#[derive(Clone)]
struct Candidate {
    tree_lazy: LazyQueryTree,

    standalone_cost: OnceCell<u64>,
    expensive_units: OnceCell<u64>,
}

impl Candidate {
    fn new(path: OperationPath, mutation_pos: MutationFieldPosition) -> Self {
        Self {
            tree_lazy: LazyTransform::new((path, mutation_pos)),
            standalone_cost: OnceCell::new(),
            expensive_units: OnceCell::new(),
        }
    }

    #[inline]
    fn get_tree<'g>(&self, graph: &Graph) -> Result<QueryTree, QueryPlanError> {
        self.tree_lazy
            .get_or_create(|(p, mp)| QueryTree::from_path(graph, &p, mp))
            .clone()
            .map_err(Into::into)
    }

    #[inline]
    fn cost_standalone(&self, graph: &Graph) -> Result<u64, QueryPlanError> {
        if let Some(v) = self.standalone_cost.get() {
            return Ok(*v);
        }
        let tree = self.get_tree(graph)?;
        let cost = calculate_cost_of_tree(graph, &tree.root);
        let _ = self.standalone_cost.set(cost);
        Ok(cost)
    }

    #[inline]
    fn units_1000(&self, graph: &Graph) -> Result<u64, QueryPlanError> {
        if let Some(v) = self.expensive_units.get() {
            return Ok(*v);
        }
        let tree = self.get_tree(graph)?;
        let units = count_expensive_units(graph, &tree.root);
        let _ = self.expensive_units.set(units);
        Ok(units)
    }
}

pub fn find_best_combination(
    graph: &Graph,
    operation: ResolvedOperation,
) -> Result<QueryTree, QueryPlanError> {
    if operation.root_field_groups.is_empty()
        || operation
            .root_field_groups
            .iter()
            .any(|paths_to_leafs| paths_to_leafs.iter().any(Vec::is_empty))
    {
        return Err(QueryPlanError::EmptyPlan);
    }

    let is_mutation = matches!(operation.operation_kind, OperationKind::Mutation);

    let mut groups: Vec<Group> = Vec::new();
    for (index, root_field_options) in operation.root_field_groups.into_iter().enumerate() {
        let mut mutation_field_position: MutationFieldPosition = None;
        if is_mutation {
            mutation_field_position = Some(index);
        }

        let leaf_groups: Vec<Group> = root_field_options
            .into_iter()
            .map(|paths_to_leaf| {
                paths_to_leaf
                    .into_iter()
                    .map(|op| Candidate::new(op, mutation_field_position))
                    .collect::<Group>()
            })
            .collect();

        groups.extend(leaf_groups);
    }

    groups.sort_by_key(|g| g.len());

    if groups.is_empty() {
        return Err(QueryPlanError::EmptyPlan);
    }

    let mut min_units_per_group: Vec<u64> = Vec::with_capacity(groups.len());
    for g in &groups {
        let mut best = u64::MAX;
        for c in g {
            let u = c.units_1000(graph)?;
            if u < best {
                best = u;
                if best == 0 {
                    break;
                }
            }
        }
        min_units_per_group.push(best);
    }
    let mut suffix_lb: Vec<u64> = vec![0; groups.len() + 1];
    for i in (0..groups.len()).rev() {
        suffix_lb[i] = suffix_lb[i + 1] + 1000 * min_units_per_group[i];
    }

    for g in &mut groups {
        g.sort_by(|a, b| {
            let ua = a.units_1000(graph).unwrap_or(u64::MAX);
            let ub = b.units_1000(graph).unwrap_or(u64::MAX);
            if ua != ub {
                return ua.cmp(&ub);
            }
            let ca = a.cost_standalone(graph).unwrap_or(u64::MAX);
            let cb = b.cost_standalone(graph).unwrap_or(u64::MAX);
            ca.cmp(&cb)
        });
    }

    let mut best_cost = u64::MAX;
    let mut best_tree: Option<QueryTree> = None;

    if let Some((c, t)) = greedy_seed(graph, &groups) {
        best_cost = c;
        best_tree = Some(t);
    }

    let mut rev_groups = groups.clone();
    rev_groups.reverse();
    if let Some((c, t)) = greedy_seed(graph, &rev_groups) {
        if c < best_cost {
            best_cost = c;
            best_tree = Some(t);
        }
    }

    let mut inter_groups = Vec::with_capacity(groups.len());
    let (mut l, mut r) = (0usize, groups.len().saturating_sub(1));
    while l <= r {
        if l == r {
            inter_groups.push(groups[l].clone());
            break;
        }
        inter_groups.push(groups[l].clone());
        inter_groups.push(groups[r].clone());
        l += 1;
        if r == 0 {
            break;
        }
        r -= 1;
    }
    if let Some((c, t)) = greedy_seed(graph, &inter_groups) {
        if c < best_cost {
            best_cost = c;
            best_tree = Some(t);
        }
    }

    let mut tt: HashMap<(usize, u64), u64> = HashMap::new();

    dfs_search(
        graph,
        &groups,
        0,
        None,
        0,
        &suffix_lb,
        &mut best_cost,
        &mut best_tree,
        0,
        &mut tt,
    )?;

    best_tree.ok_or(QueryPlanError::EmptyPlan)
}

fn greedy_seed(graph: &Graph, groups: &[Group]) -> Option<(u64, QueryTree)> {
    let mut current_tree: Option<QueryTree> = None;
    let mut current_cost: u64 = 0;

    for g in groups {
        let mut best_delta = u64::MAX;
        let mut best_next: Option<(u64, QueryTree)> = None;

        for c in g {
            let cand_tree = match c.get_tree(graph) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let next_tree = match current_tree.as_ref() {
                Some(t) => {
                    let mut merged = t.clone();
                    Arc::make_mut(&mut merged.root).merge_nodes(&cand_tree.root);
                    merged
                }
                None => cand_tree.clone(),
            };
            let next_cost = calculate_cost_of_tree(graph, &next_tree.root);
            let delta = next_cost.saturating_sub(current_cost);
            if delta < best_delta {
                best_delta = delta;
                best_next = Some((next_cost, next_tree));
            }
        }

        if let Some((nc, nt)) = best_next {
            current_cost = nc;
            current_tree = Some(nt);
        } else {
            return None;
        }
    }

    current_tree.map(|t| (current_cost, t))
}

fn dfs_search(
    graph: &Graph,
    groups: &[Group],
    idx: usize,
    tree_so_far: Option<QueryTree>,
    cost_so_far: u64,
    suffix_lb: &[u64],
    best_cost: &mut u64,
    best_tree: &mut Option<QueryTree>,
    sig: u64,
    tt: &mut HashMap<(usize, u64), u64>,
) -> Result<(), QueryPlanError> {
    if cost_so_far + suffix_lb[idx] >= *best_cost {
        return Ok(());
    }

    if idx == groups.len() {
        if cost_so_far < *best_cost {
            *best_cost = cost_so_far;
            *best_tree = tree_so_far;
        }
        return Ok(());
    }

    if let Some(&seen) = tt.get(&(idx, sig)) {
        if cost_so_far >= seen {
            return Ok(());
        }
    }
    tt.insert((idx, sig), cost_so_far);

    for (cand_idx, cand) in groups[idx].iter().enumerate() {
        let cand_tree = cand.get_tree(graph)?;

        let next_tree = match tree_so_far.as_ref() {
            Some(t) => {
                let mut merged = t.clone();
                Arc::make_mut(&mut merged.root).merge_nodes(&cand_tree.root);
                merged
            }
            None => cand_tree.clone(),
        };

        let next_cost = calculate_cost_of_tree(graph, &next_tree.root);
        if next_cost >= *best_cost {
            continue;
        }

        let mut hasher = Xxh3::new();
        hasher.update(&sig.to_le_bytes());
        hasher.update(&(idx as u64).to_le_bytes());
        hasher.update(&(cand_idx as u64).to_le_bytes());
        let next_sig = hasher.digest();

        dfs_search(
            graph,
            groups,
            idx + 1,
            Some(next_tree),
            next_cost,
            suffix_lb,
            best_cost,
            best_tree,
            next_sig,
            tt,
        )?;
    }

    Ok(())
}

fn calculate_cost_of_tree(graph: &Graph, node: &QueryTreeNode) -> u64 {
    let mut current_cost: u64 = 1;

    for child in &node.children {
        if child.edge_from_parent.is_some_and(|edge_index| {
            matches!(
                graph.edge(edge_index).expect("to find an edge"),
                Edge::SubgraphEntrypoint { .. }
            )
        }) {
            current_cost += 1000;
        }

        current_cost += calculate_cost_of_tree(graph, child);
    }

    for req in &node.requirements {
        current_cost += 1000;
        current_cost += calculate_cost_of_tree(graph, req);
    }

    current_cost
}

fn count_expensive_units(graph: &Graph, node: &QueryTreeNode) -> u64 {
    let mut units: u64 = 0;

    for child in &node.children {
        if child.edge_from_parent.is_some_and(|edge_index| {
            matches!(
                graph.edge(edge_index).expect("to find an edge"),
                Edge::SubgraphEntrypoint { .. }
            )
        }) {
            units += 1;
        }
        units += count_expensive_units(graph, child);
    }

    for req in &node.requirements {
        units += 1;
        units += count_expensive_units(graph, req);
    }

    units
}
