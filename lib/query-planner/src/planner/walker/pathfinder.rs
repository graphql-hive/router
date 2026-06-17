use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::visit::{EdgeRef, NodeRef};
use tracing::{instrument, trace};

use crate::ast::merge_path::Condition;
use crate::ast::selection_set::InlineFragmentSelection;
use crate::graph::edge::PlannerOverrideContext;
use crate::utils::cancellation::CancellationToken;
use crate::{
    ast::{
        selection_item::SelectionItem, selection_set::FieldSelection,
        type_aware_selection::TypeAwareSelection,
    },
    graph::{
        edge::{Edge, EdgeReference},
        Graph,
    },
    planner::{
        tree::query_tree_node::QueryTreeNode,
        walker::best_path::{find_best_paths, BestPathTracker},
    },
};

use super::{error::WalkOperationError, excluded::ExcludedFromLookup, path::OperationPath};

pub type VisitedGraphs<'graph> = HashSet<&'graph str>;
type ActiveEdgeChecks = HashSet<(NodeIndex, EdgeIndex)>;

struct IndirectPathsLookupQueue<'graph> {
    queue: Vec<(
        VisitedGraphs<'graph>,
        HashSet<TypeAwareSelection>,
        OperationPath<'graph>,
    )>,
}

impl<'graph> IndirectPathsLookupQueue<'graph> {
    pub fn new_from_excluded(
        excluded: &ExcludedFromLookup<'graph>,
        path: &OperationPath<'graph>,
    ) -> Self {
        IndirectPathsLookupQueue {
            queue: vec![(
                excluded.graph_ids.clone(),
                excluded
                    .requirement
                    .clone()
                    .into_iter()
                    .collect::<HashSet<_>>(),
                path.clone(),
            )],
        }
    }

    pub fn add(
        &mut self,
        visited_graphs: VisitedGraphs<'graph>,
        selections: HashSet<TypeAwareSelection>,
        path: OperationPath<'graph>,
    ) {
        self.queue.push((visited_graphs, selections, path));
    }

    pub fn pop(
        &mut self,
    ) -> Option<(
        VisitedGraphs<'graph>,
        HashSet<TypeAwareSelection>,
        OperationPath<'graph>,
    )> {
        self.queue.pop()
    }
}

#[derive(Debug)]
pub enum NavigationTarget<'op> {
    Field(&'op FieldSelection),
    ConcreteType(&'op str, Option<Condition>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum NavigationTargetKey<'op> {
    Field(&'op str),
    ConcreteType(&'op str),
}

impl<'op> From<&'op NavigationTarget<'op>> for NavigationTargetKey<'op> {
    fn from(target: &'op NavigationTarget<'op>) -> Self {
        match target {
            NavigationTarget::Field(field) => NavigationTargetKey::Field(&field.name),
            NavigationTarget::ConcreteType(type_name, _) => {
                NavigationTargetKey::ConcreteType(type_name)
            }
        }
    }
}

struct PathSearch<'graph> {
    graph: &'graph Graph,
    override_context: &'graph PlannerOverrideContext,
    cancellation_token: &'graph CancellationToken,
    /// Edges currently being checked in this path search.
    /// Used to stop recursive loops when an edge depends on itself.
    active_edge_checks: ActiveEdgeChecks,
}

impl<'graph> PathSearch<'graph> {
    fn new(
        graph: &'graph Graph,
        override_context: &'graph PlannerOverrideContext,
        cancellation_token: &'graph CancellationToken,
    ) -> Self {
        Self {
            graph,
            override_context,
            cancellation_token,
            active_edge_checks: ActiveEdgeChecks::new(),
        }
    }
}

#[instrument(level = "trace", skip_all, fields(
  path = path.pretty_print(graph),
  current_cost = path.cost
))]
pub fn find_indirect_paths<'graph>(
    graph: &'graph Graph,
    override_context: &'graph PlannerOverrideContext,
    path: &OperationPath<'graph>,
    target: &NavigationTarget<'_>,
    excluded: &ExcludedFromLookup<'graph>,
    cancellation_token: &'graph CancellationToken,
) -> Result<Vec<OperationPath<'graph>>, WalkOperationError> {
    PathSearch::new(graph, override_context, cancellation_token)
        .find_indirect_paths(path, target, excluded)
}

impl<'graph> PathSearch<'graph> {
    fn find_indirect_paths(
        &mut self,
        path: &OperationPath<'graph>,
        target: &NavigationTarget<'_>,
        excluded: &ExcludedFromLookup<'graph>,
    ) -> Result<Vec<OperationPath<'graph>>, WalkOperationError> {
        let graph = self.graph;
        let cancellation_token = self.cancellation_token;
        let mut tracker = BestPathTracker::new(graph);
        let mut seen = HashSet::new();
        let target_key = NavigationTargetKey::from(target);
        let tail_node_index = path.tail();
        let tail_node = graph.node(tail_node_index)?;
        let source_graph_id = tail_node
            .graph_id()
            .ok_or(WalkOperationError::TailMissingInfo(tail_node_index))?;

        // Respect the path's current union scope when targeting a concrete type.
        if let NavigationTarget::ConcreteType(type_name, _) = target {
            if !path.can_resolve_union_member(type_name) {
                return Ok(Vec::new());
            }
        }

        let mut queue = IndirectPathsLookupQueue::new_from_excluded(excluded, path);

        while let Some(item) = queue.pop() {
            cancellation_token.bail_if_cancelled()?;
            let (visited_graphs, visited_key_fields, path) = item;

            if !seen.insert((path.tail(), target_key)) {
                trace!(
                    "Ignoring. Already searched this path tail for this target: {}",
                    path.pretty_print(graph)
                );
                continue;
            }

            let relevant_edges = graph.edges_from(path.tail()).filter(|e| {
                matches!(
                    e.weight(),
                    Edge::EntityMove { .. } | Edge::InterfaceObjectTypeMove { .. }
                )
            });

            for edge_ref in relevant_edges {
                trace!(
                    "Exploring edge {}",
                    graph.pretty_print_edge(edge_ref.id(), false)
                );

                let edge_tail_graph_id = graph.node(edge_ref.target().id())?.graph_id().unwrap();

                if visited_graphs.contains(edge_tail_graph_id) {
                    trace!(
                    "Ignoring, graph is excluded and already visited (current: {}, visited: {:?})",
                    edge_tail_graph_id,
                    visited_graphs
                );
                    continue;
                }

                let edge_tail_graph_id = graph.node(edge_ref.target().id())?.graph_id().unwrap();
                let edge = edge_ref.weight();

                if edge_tail_graph_id == source_graph_id
                    && !matches!(edge, Edge::InterfaceObjectTypeMove(..))
                {
                    // Prevent a situation where we are going back to the same graph
                    // The only exception is when we are moving to an abstract type
                    trace!("Ignoring. We would go back to the same graph");
                    continue;
                }

                // A huge win for performance, is when you do less work :D
                // We can ignore an edge that has already been visited with the same key fields / requirements.
                // The way entity-move edges are created, where every graph points to every other graph:
                //  Graph A: User @key(id) @key(name)
                //  Graph B: User @key(id)
                //  Edges in a merged graph:
                //    - User/A @key(id) -> User/B
                //    - User/B @key(id) -> User/A
                //    - User/B @key(name) -> User/A
                // Allows us to ignore an edge with the same key fields.
                // That's because in some other path, we will or already have checked the other edge.
                let requirements_already_checked = match edge.requirements() {
                    Some(selection_requirements) => {
                        visited_key_fields.contains(selection_requirements)
                    }
                    None => false,
                };

                if requirements_already_checked {
                    trace!("Ignoring. Already visited similar edge");
                    continue;
                }

                let mut new_excluded_graph_ids = visited_graphs.clone();
                new_excluded_graph_ids.insert(edge_tail_graph_id);
                let new_excluded = ExcludedFromLookup {
                    graph_ids: new_excluded_graph_ids,
                    requirement: visited_key_fields.clone(),
                };

                let can_be_satisfied =
                    self.can_satisfy_edge(&edge_ref, &path, &new_excluded, false)?;

                match can_be_satisfied {
                    None => {
                        trace!("Requirements not satisfied, continue look up...");
                        continue;
                    }
                    Some(paths) => {
                        trace!(
                            "Advancing path to {}",
                            graph.pretty_print_edge(edge_ref.id(), false)
                        );

                        let next_resolution_path = path.advance(
                            graph,
                            &edge_ref,
                            QueryTreeNode::from_paths(graph, &paths, None)?,
                            target,
                        );

                        let direct_paths = self.find_direct_paths(&next_resolution_path, target)?;

                        if !direct_paths.is_empty() {
                            trace!(
                                "Found {} direct paths to {}",
                                direct_paths.len(),
                                graph.pretty_print_edge(edge_ref.id(), false)
                            );

                            for direct_path in direct_paths {
                                tracker.add(&direct_path)?;
                            }

                            continue;
                        } else {
                            trace!("No direct paths found");

                            let mut new_visited_graphs = visited_graphs.clone();
                            new_visited_graphs.insert(edge_tail_graph_id);

                            let next_requirements = match edge.requirements() {
                                Some(requirements) => {
                                    let mut new_visited_key_fields = visited_key_fields.clone();
                                    new_visited_key_fields.insert(requirements.clone());
                                    new_visited_key_fields
                                }
                                None => visited_key_fields.clone(),
                            };

                            queue.add(new_visited_graphs, next_requirements, next_resolution_path);

                            trace!("going deeper");
                        }
                    }
                }
            }
        }

        let best_paths = tracker.get_best_paths();

        trace!(
            "Finished finding indirect paths, found total of {}",
            best_paths.len()
        );

        // TODO: this should be done in a more efficient way, like I do in the satisfiability checker
        // I set shortest path right after each path is generated

        Ok(best_paths)
    }
}

impl<'graph> PathSearch<'graph> {
    fn try_advance_direct_path(
        &mut self,
        path: &OperationPath<'graph>,
        edge_ref: &EdgeReference<'graph>,
        target: &NavigationTarget<'_>,
    ) -> Result<Option<OperationPath<'graph>>, WalkOperationError> {
        let graph = self.graph;
        trace!(
            "Checking edge {}",
            graph.pretty_print_edge(edge_ref.id(), false)
        );

        let can_be_satisfied =
            self.can_satisfy_edge(edge_ref, path, &ExcludedFromLookup::new(), false)?;

        match can_be_satisfied {
            Some(paths) => {
                trace!(
                    "Advancing path {} with edge {}",
                    path.pretty_print(graph),
                    graph.pretty_print_edge(edge_ref.id(), false)
                );

                let next_resolution_path = path.advance(
                    graph,
                    edge_ref,
                    QueryTreeNode::from_paths(graph, &paths, None)?,
                    target,
                );

                Ok(Some(next_resolution_path))
            }
            None => {
                trace!("Edge not satisfied, continue look up...");
                Ok(None)
            }
        }
    }
}

pub fn find_self_referencing_direct_path<'graph>(
    graph: &'graph Graph,
    override_context: &'graph PlannerOverrideContext,
    path: &OperationPath<'graph>,
    type_name: &'graph str,
    condition: &Condition,
    cancellation_token: &'graph CancellationToken,
) -> Result<OperationPath<'graph>, WalkOperationError> {
    let path_tail_index = path.tail();
    let mut path_search = PathSearch::new(graph, override_context, cancellation_token);

    for edge_ref in graph
        .edges_from(path_tail_index)
        .filter(move |e| match e.weight() {
            Edge::Selfie(t) => t == type_name,
            _ => false,
        })
    {
        if let Some(new_path) = path_search.try_advance_direct_path(
            path,
            &edge_ref,
            &NavigationTarget::ConcreteType(type_name, Some(condition.clone())),
        )? {
            trace!("Finished finding direct path, found one",);
            return Ok(new_path);
        }
    }

    trace!("Finished finding direct path, found none",);

    Err(WalkOperationError::NoPathsFound(type_name.to_string()))
}

#[instrument(level = "trace", skip_all, fields(
    path = path.pretty_print(graph),
    current_cost = path.cost,
))]
pub fn find_direct_paths<'graph>(
    graph: &'graph Graph,
    override_context: &'graph PlannerOverrideContext,
    path: &OperationPath<'graph>,
    target: &NavigationTarget<'_>,
    cancellation_token: &'graph CancellationToken,
) -> Result<Vec<OperationPath<'graph>>, WalkOperationError> {
    PathSearch::new(graph, override_context, cancellation_token).find_direct_paths(path, target)
}

impl<'graph> PathSearch<'graph> {
    fn find_direct_paths(
        &mut self,
        path: &OperationPath<'graph>,
        target: &NavigationTarget<'_>,
    ) -> Result<Vec<OperationPath<'graph>>, WalkOperationError> {
        let graph = self.graph;
        let mut result: Vec<OperationPath<'graph>> = vec![];
        let path_tail_index = path.tail();

        // Respect the path's current union scope when targeting a concrete type.
        if let NavigationTarget::ConcreteType(type_name, _) = target {
            if !path.can_resolve_union_member(type_name) {
                return Ok(result);
            }
        }

        let edges_iter: Box<dyn Iterator<Item = _>> = match target {
            NavigationTarget::Field(field) => {
                Box::new(graph.edges_from(path_tail_index).filter(
                    move |e| matches!(e.weight(), Edge::FieldMove(f) if f.name == field.name),
                ))
            }
            NavigationTarget::ConcreteType(type_name, _condition) => Box::new(
                graph
                    .edges_from(path_tail_index)
                    .filter(move |e| match e.weight() {
                        Edge::AbstractMove(t) => t == type_name,
                        Edge::InterfaceObjectTypeMove(t) => &t.object_type_name == type_name,
                        _ => false,
                    }),
            ),
        };

        for edge_ref in edges_iter {
            if let Some(new_path) = self.try_advance_direct_path(path, &edge_ref, target)? {
                result.push(new_path);
            }
        }

        trace!(
            "Finished finding direct paths, found total of {}",
            result.len()
        );

        Ok(result)
    }
}

#[instrument(level = "trace", skip_all, fields(
  path = path.pretty_print(graph),
  edge = edge_ref.weight().display_name(),
))]
pub fn can_satisfy_edge<'graph>(
    graph: &'graph Graph,
    override_context: &'graph PlannerOverrideContext,
    edge_ref: &EdgeReference<'graph>,
    path: &OperationPath<'graph>,
    excluded: &ExcludedFromLookup<'graph>,
    use_only_direct_edges: bool,
    cancellation_token: &'graph CancellationToken,
) -> Result<Option<Vec<OperationPath<'graph>>>, WalkOperationError> {
    PathSearch::new(graph, override_context, cancellation_token).can_satisfy_edge(
        edge_ref,
        path,
        excluded,
        use_only_direct_edges,
    )
}

impl<'graph> PathSearch<'graph> {
    fn can_satisfy_edge(
        &mut self,
        edge_ref: &EdgeReference<'graph>,
        path: &OperationPath<'graph>,
        excluded: &ExcludedFromLookup<'graph>,
        use_only_direct_edges: bool,
    ) -> Result<Option<Vec<OperationPath<'graph>>>, WalkOperationError> {
        let graph = self.graph;
        let active_key = (path.tail(), edge_ref.id());
        if !self.active_edge_checks.insert(active_key) {
            trace!(
                "Ignoring. Already trying to satisfy edge '{}' from this path tail: {}",
                graph.pretty_print_edge(edge_ref.id(), false),
                path.pretty_print(graph)
            );
            return Ok(None);
        }

        let result = self.check_edge_requirements(edge_ref, path, excluded, use_only_direct_edges);

        self.active_edge_checks.remove(&active_key);

        result
    }

    fn check_edge_requirements(
        &mut self,
        edge_ref: &EdgeReference<'graph>,
        path: &OperationPath<'graph>,
        excluded: &ExcludedFromLookup<'graph>,
        use_only_direct_edges: bool,
    ) -> Result<Option<Vec<OperationPath<'graph>>>, WalkOperationError> {
        let graph = self.graph;
        let override_context = self.override_context;
        let cancellation_token = self.cancellation_token;
        let edge = edge_ref.weight();

        if let Edge::FieldMove(field_move) = edge {
            // TODO: This should be passed from the executor,
            //       I will work on it next.
            if !field_move.satisfies_override_rules(override_context) {
                return Ok(None);
            }
        }

        match edge.requirements() {
            None => Ok(Some(vec![])),
            Some(selections) => {
                trace!(
                    "checking requirements {} for edge '{}'",
                    selections,
                    graph.pretty_print_edge(edge_ref.id(), false)
                );

                let mut requirements: VecDeque<MoveRequirement> = VecDeque::new();
                let mut paths_to_requirements: Vec<OperationPath<'graph>> = vec![];

                for selection in selections.selection_set.items.iter() {
                    requirements.push_front(MoveRequirement {
                        paths: Rc::new(vec![path.clone()]),
                        selection: selection.clone(),
                    });
                }

                // it's important to pop from the end as we want to process the last added requirement first
                while let Some(requirement) = requirements.pop_back() {
                    cancellation_token.bail_if_cancelled()?;
                    match &requirement.selection {
                        SelectionItem::Field(selection_field_requirement) => {
                            let result = self.validate_field_requirement(
                                &requirement,
                                selection_field_requirement,
                                excluded,
                                use_only_direct_edges,
                            )?;

                            match result {
                                Some((next_paths, next_requirements)) => {
                                    trace!("Paths for {}", selection_field_requirement);

                                    for next_path in next_paths.iter() {
                                        trace!("  Path {} is valid", next_path.pretty_print(graph));
                                    }

                                    if selection_field_requirement.is_leaf() {
                                        let best_paths = find_best_paths(next_paths);
                                        trace!(
                                            "Found {} best paths for this leaf requirement",
                                            best_paths.len()
                                        );

                                        for best_path in best_paths {
                                            paths_to_requirements.push(
                                                path.build_requirement_continuation_path(
                                                    &best_path,
                                                ),
                                            );
                                        }
                                    }

                                    for req in next_requirements.into_iter().rev() {
                                        requirements.push_front(req);
                                    }
                                }
                                None => {
                                    return Ok(None);
                                }
                            };
                        }
                        SelectionItem::InlineFragment(fragment_selection) => {
                            let fragment_requirements = self.validate_fragment_requirement(
                                &requirement,
                                fragment_selection,
                                excluded,
                            )?;

                            match fragment_requirements {
                                Some((next_paths, next_requirements)) => {
                                    trace!("Paths for {}", fragment_selection);

                                    for next_path in next_paths.iter() {
                                        trace!("  Path {} is valid", next_path.pretty_print(graph));
                                    }

                                    for req in next_requirements.into_iter().rev() {
                                        requirements.push_front(req);
                                    }
                                }
                                None => {
                                    return Ok(None);
                                }
                            };
                        }
                        SelectionItem::FragmentSpread(_) => {
                            // No processing needed for FragmentSpread
                        }
                    }
                }

                for path in paths_to_requirements.iter() {
                    trace!("path {} is valid", path.pretty_print(graph));
                }

                Ok(Some(paths_to_requirements))
            }
        }
    }
}

#[derive(Debug)]
pub struct MoveRequirement<'graph> {
    pub paths: Rc<Vec<OperationPath<'graph>>>,
    pub selection: SelectionItem,
}

type FieldRequirementsResult<'graph> =
    Option<(Vec<OperationPath<'graph>>, Vec<MoveRequirement<'graph>>)>;
type FragmentRequirementsResult<'graph> =
    Option<(Vec<OperationPath<'graph>>, Vec<MoveRequirement<'graph>>)>;

impl<'graph> PathSearch<'graph> {
    #[instrument(level = "trace", skip_all, fields(field = field.name))]
    fn validate_field_requirement(
        &mut self,
        move_requirement: &MoveRequirement<'graph>,
        field: &FieldSelection,
        excluded: &ExcludedFromLookup<'graph>,
        use_only_direct_edges: bool,
    ) -> Result<FieldRequirementsResult<'graph>, WalkOperationError> {
        let mut direct_path_results: Vec<Vec<OperationPath<'graph>>> =
            Vec::with_capacity(move_requirement.paths.len());
        let mut indirect_path_results: Vec<Vec<OperationPath<'graph>>> =
            Vec::with_capacity(move_requirement.paths.len());

        for path in move_requirement.paths.iter() {
            let direct_paths = self.find_direct_paths(path, &NavigationTarget::Field(field))?;
            // Skip looking for indirect paths if we already found direct paths to a leaf
            let found_direct_paths_to_leaf = !direct_paths.is_empty() && field.is_leaf();
            direct_path_results.push(direct_paths);

            let needs_indirect = !use_only_direct_edges && !found_direct_paths_to_leaf;
            let indirect_paths = if needs_indirect {
                self.find_indirect_paths(path, &NavigationTarget::Field(field), excluded)?
            } else {
                Vec::new()
            };

            indirect_path_results.push(indirect_paths);
        }

        // sum of direct and indirect
        let total_capacity: usize = direct_path_results.iter().map(|v| v.len()).sum::<usize>()
            + indirect_path_results.iter().map(|v| v.len()).sum::<usize>();

        let mut next_paths: Vec<OperationPath<'graph>> = Vec::with_capacity(total_capacity);

        // These extend calls should not reallocate `next_paths`.
        for paths_vec in direct_path_results {
            next_paths.extend(paths_vec);
        }
        // No need to check use_only_direct_edges again, indirect_path_results_vecs will be empty if not used.
        for paths_vec in indirect_path_results {
            next_paths.extend(paths_vec);
        }

        if next_paths.is_empty() {
            return Ok(None);
        }

        if move_requirement.selection.selections().is_none()
            || move_requirement
                .selection
                .selections()
                .is_some_and(|s| s.is_empty())
        {
            // No sub-selections, next_paths is returned directly.
            return Ok(Some((next_paths, vec![])));
        }

        let shared_next_paths_for_subs = Rc::new(next_paths.clone());
        let next_requirements: Vec<MoveRequirement<'graph>> = move_requirement
            .selection
            .selections()
            .unwrap() // Safe due to the check above
            .iter()
            .map(|selection_item| MoveRequirement {
                selection: selection_item.clone(),
                paths: Rc::clone(&shared_next_paths_for_subs),
            })
            .collect();

        Ok(Some((next_paths, next_requirements)))
    }
}

impl<'graph> PathSearch<'graph> {
    #[instrument(level = "trace", skip_all, fields(type_condition = fragment_selection.type_condition))]
    fn validate_fragment_requirement(
        &mut self,
        requirement: &MoveRequirement<'graph>,
        fragment_selection: &InlineFragmentSelection,
        excluded: &ExcludedFromLookup<'graph>,
    ) -> Result<FragmentRequirementsResult<'graph>, WalkOperationError> {
        let type_name = &fragment_selection.type_condition;
        // Collect all Vec<OperationPath<'graph>> results from find_direct_paths
        let mut direct_path_results: Vec<Vec<OperationPath<'graph>>> =
            Vec::with_capacity(requirement.paths.len());
        for path in requirement.paths.iter() {
            direct_path_results.push(self.find_direct_paths(
                path,
                // @skip/@include can't be used in @requires and @provides,
                // that's why we pass no condition
                &NavigationTarget::ConcreteType(type_name, None),
            )?);
        }

        // Collect all Vec<OperationPath<'graph>> results from find_indirect_paths
        let mut indirect_path_results: Vec<Vec<OperationPath<'graph>>> =
            Vec::with_capacity(requirement.paths.len());
        for path_from_rc in requirement.paths.iter() {
            indirect_path_results.push(self.find_indirect_paths(
                path_from_rc,
                // @skip/@include can't be used in @requires and @provides,
                // that's why we pass no condition
                &NavigationTarget::ConcreteType(type_name, None),
                excluded,
            )?);
        }

        // sum of direct and indirect
        let total_capacity: usize = direct_path_results.iter().map(|v| v.len()).sum::<usize>()
            + indirect_path_results.iter().map(|v| v.len()).sum::<usize>();

        let mut next_paths: Vec<OperationPath<'graph>> = Vec::with_capacity(total_capacity);

        // These extend calls should not reallocate `next_paths`.
        for paths_vec in direct_path_results {
            next_paths.extend(paths_vec);
        }
        for paths_vec in indirect_path_results {
            next_paths.extend(paths_vec);
        }

        if next_paths.is_empty() {
            return Ok(None);
        }

        if requirement.selection.selections().is_none()
            || requirement
                .selection
                .selections()
                .is_some_and(|s| s.is_empty())
        {
            // No sub-selections, next_paths is returned directly.
            return Ok(Some((next_paths, vec![])));
        }

        let shared_next_paths_for_subs = Rc::new(next_paths.clone());
        let next_requirements: Vec<MoveRequirement<'graph>> = requirement
            .selection
            .selections()
            .unwrap() // Safe due to the check above
            .iter()
            .map(|selection_item| MoveRequirement {
                selection: selection_item.clone(),
                paths: Rc::clone(&shared_next_paths_for_subs),
            })
            .collect();

        Ok(Some((next_paths, next_requirements)))
    }
}
