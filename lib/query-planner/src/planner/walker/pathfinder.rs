use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

use petgraph::visit::{EdgeRef, NodeRef};
use tracing::{instrument, trace};

use crate::ast::merge_path::Condition;
use crate::ast::selection_set::InlineFragmentSelection;
use crate::graph::edge::PlannerOverrideContext;
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

pub type VisitedGraphs = HashSet<String>;

struct IndirectPathsLookupQueue {
    queue: Vec<(VisitedGraphs, HashSet<TypeAwareSelection>, OperationPath)>,
}

impl IndirectPathsLookupQueue {
    pub fn new_from_excluded(excluded: &ExcludedFromLookup, path: &OperationPath) -> Self {
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
        visited_graphs: VisitedGraphs,
        selections: HashSet<TypeAwareSelection>,
        path: OperationPath,
    ) {
        self.queue.push((visited_graphs, selections, path));
    }

    pub fn pop(&mut self) -> Option<(VisitedGraphs, HashSet<TypeAwareSelection>, OperationPath)> {
        self.queue.pop()
    }
}

#[derive(Debug)]
pub enum NavigationTarget<'a> {
    Field(&'a FieldSelection),
    ConcreteType(&'a str, Option<Condition>),
}

#[instrument(level = "trace",skip(graph, override_context, excluded, target), fields(
  path = path.pretty_print(graph),
  current_cost = path.cost
))]
pub fn find_indirect_paths(
    graph: &Graph,
    override_context: &PlannerOverrideContext,
    path: &OperationPath,
    target: &NavigationTarget,
    excluded: &ExcludedFromLookup,
) -> Result<Vec<OperationPath>, WalkOperationError> {
    let mut tracker = BestPathTracker::new(graph);
    let tail_node_index = path.tail();
    let tail_node = graph.node(tail_node_index)?;
    let source_graph_id = tail_node
        .graph_id()
        .ok_or(WalkOperationError::TailMissingInfo(tail_node_index))?;

    let mut queue = IndirectPathsLookupQueue::new_from_excluded(excluded, path);

    while let Some(item) = queue.pop() {
        let (visited_graphs, visited_key_fields, path) = item;

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
                Some(selection_requirements) => visited_key_fields.contains(selection_requirements),
                None => false,
            };

            if requirements_already_checked {
                trace!("Ignoring. Already visited similar edge");
                continue;
            }

            let new_excluded = excluded.next(edge_tail_graph_id, &visited_key_fields);

            let can_be_satisfied = can_satisfy_edge(
                graph,
                override_context,
                &edge_ref,
                &path,
                &new_excluded,
                false,
            )?;

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
                        &edge_ref,
                        QueryTreeNode::from_paths(graph, &paths, None)?,
                        target,
                    );

                    let direct_paths =
                        find_direct_paths(graph, override_context, &next_resolution_path, target)?;

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
                        new_visited_graphs.insert(edge_tail_graph_id.to_string());

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

fn try_advance_direct_path<'a>(
    graph: &'a Graph,
    path: &OperationPath,
    override_context: &PlannerOverrideContext,
    edge_ref: &EdgeReference,
    target: &NavigationTarget<'a>,
) -> Result<Option<OperationPath>, WalkOperationError> {
    trace!(
        "Checking edge {}",
        graph.pretty_print_edge(edge_ref.id(), false)
    );

    let can_be_satisfied = can_satisfy_edge(
        graph,
        override_context,
        edge_ref,
        path,
        &ExcludedFromLookup::new(),
        false,
    )?;

    match can_be_satisfied {
        Some(paths) => {
            trace!(
                "Advancing path {} with edge {}",
                path.pretty_print(graph),
                graph.pretty_print_edge(edge_ref.id(), false)
            );

            let next_resolution_path = path.advance(
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

#[instrument(level = "trace",skip(graph, target, override_context), fields(
    path = path.pretty_print(graph),
    current_cost = path.cost,
))]
pub fn find_direct_paths(
    graph: &Graph,
    override_context: &PlannerOverrideContext,
    path: &OperationPath,
    target: &NavigationTarget,
) -> Result<Vec<OperationPath>, WalkOperationError> {
    let mut result: Vec<OperationPath> = vec![];
    let path_tail_index = path.tail();

    let edges_iter: Box<dyn Iterator<Item = _>> = match target {
        NavigationTarget::Field(field) => Box::new(
            graph
                .edges_from(path_tail_index)
                .filter(move |e| matches!(e.weight(), Edge::FieldMove(f) if f.name == field.name)),
        ),
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
        if let Some(new_path) =
            try_advance_direct_path(graph, path, override_context, &edge_ref, target)?
        {
            result.push(new_path);
        }
    }

    trace!(
        "Finished finding direct paths, found total of {}",
        result.len()
    );

    Ok(result)
}

#[instrument(level = "trace",skip_all, fields(
  path = path.pretty_print(graph),
  edge = edge_ref.weight().display_name(),
))]
pub fn can_satisfy_edge(
    graph: &Graph,
    override_context: &PlannerOverrideContext,
    edge_ref: &EdgeReference,
    path: &OperationPath,
    excluded: &ExcludedFromLookup,
    use_only_direct_edges: bool,
) -> Result<Option<Vec<OperationPath>>, WalkOperationError> {
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
            let mut paths_to_requirements: Vec<OperationPath> = vec![];

            for selection in selections.selection_set.items.iter() {
                requirements.push_front(MoveRequirement {
                    paths: Rc::new(vec![path.clone()]),
                    selection: selection.clone(),
                });
            }

            // it's important to pop from the end as we want to process the last added requirement first
            while let Some(requirement) = requirements.pop_back() {
                match &requirement.selection {
                    SelectionItem::Field(selection_field_requirement) => {
                        let result = validate_field_requirement(
                            graph,
                            override_context,
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
                                            path.build_requirement_continuation_path(&best_path),
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
                        let fragment_requirements = validate_fragment_requirement(
                            graph,
                            override_context,
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

#[derive(Debug)]
pub struct MoveRequirement {
    pub paths: Rc<Vec<OperationPath>>,
    pub selection: SelectionItem,
}

type FieldRequirementsResult = Option<(Vec<OperationPath>, Vec<MoveRequirement>)>;
type FragmentRequirementsResult = Option<(Vec<OperationPath>, Vec<MoveRequirement>)>;

#[instrument(level = "trace", skip_all, fields(field = field.name))]
fn validate_field_requirement(
    graph: &Graph,
    override_context: &PlannerOverrideContext,
    move_requirement: &MoveRequirement, // Contains Rc<Vec<OperationPath>>
    field: &FieldSelection,
    excluded: &ExcludedFromLookup,
    use_only_direct_edges: bool,
) -> Result<FieldRequirementsResult, WalkOperationError> {
    // Collect all Vec<OperationPath> results from find_direct_paths
    let mut direct_path_results: Vec<Vec<OperationPath>> =
        Vec::with_capacity(move_requirement.paths.len());
    for path in move_requirement.paths.iter() {
        direct_path_results.push(find_direct_paths(
            graph,
            override_context,
            path,
            &NavigationTarget::Field(field),
        )?);
    }

    // Collect all Vec<OperationPath> results from find_indirect_paths, if needed
    let indirect_path_results: Vec<Vec<OperationPath>> = if !use_only_direct_edges {
        let mut temp_indirect_results = Vec::with_capacity(move_requirement.paths.len());
        for path_from_rc in move_requirement.paths.iter() {
            temp_indirect_results.push(find_indirect_paths(
                graph,
                override_context,
                path_from_rc,
                &NavigationTarget::Field(field),
                excluded,
            )?);
        }
        temp_indirect_results
    } else {
        Vec::new()
    };

    // sum of direct and indirect
    let total_capacity: usize = direct_path_results.iter().map(|v| v.len()).sum::<usize>()
        + indirect_path_results.iter().map(|v| v.len()).sum::<usize>();

    let mut next_paths: Vec<OperationPath> = Vec::with_capacity(total_capacity);

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
    let next_requirements: Vec<MoveRequirement> = move_requirement
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

#[instrument(level = "trace", skip_all, fields(type_condition = fragment_selection.type_condition))]
fn validate_fragment_requirement(
    graph: &Graph,
    override_context: &PlannerOverrideContext,
    requirement: &MoveRequirement,
    fragment_selection: &InlineFragmentSelection,
    excluded: &ExcludedFromLookup,
) -> Result<FragmentRequirementsResult, WalkOperationError> {
    let type_name = &fragment_selection.type_condition;
    // Collect all Vec<OperationPath> results from find_direct_paths
    let mut direct_path_results: Vec<Vec<OperationPath>> =
        Vec::with_capacity(requirement.paths.len());
    for path in requirement.paths.iter() {
        direct_path_results.push(find_direct_paths(
            graph,
            override_context,
            path,
            // @skip/@include can't be used in @requires and @provides,
            // that's why we pass no condition
            &NavigationTarget::ConcreteType(type_name, None),
        )?);
    }

    // Collect all Vec<OperationPath> results from find_indirect_paths
    let mut indirect_path_results: Vec<Vec<OperationPath>> =
        Vec::with_capacity(requirement.paths.len());
    for path_from_rc in requirement.paths.iter() {
        indirect_path_results.push(find_indirect_paths(
            graph,
            override_context,
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

    let mut next_paths: Vec<OperationPath> = Vec::with_capacity(total_capacity);

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
    let next_requirements: Vec<MoveRequirement> = requirement
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
