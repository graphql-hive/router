use std::collections::{HashSet, VecDeque};

use bumpalo::collections::Vec as BumpVec;
use petgraph::visit::{EdgeRef, NodeRef};
use tracing::{instrument, trace};

use crate::ast::selection_set::InlineFragmentSelection;
use crate::planner::tree::query_tree_node::QueryTreeNode;
use crate::{
    ast::{
        selection_item::SelectionItem, selection_set::FieldSelection,
        type_aware_selection::TypeAwareSelection,
    },
    graph::edge::{Edge, EdgeReference},
    planner::walker::best_path::{find_best_paths, BestPathTracker},
};

use super::{
    error::WalkOperationError, excluded::ExcludedFromLookup, path::OperationPath, WalkContext,
};

pub type VisitedGraphs = HashSet<String>;

struct IndirectPathsLookupQueue<'bump, 'a> {
    queue: BumpVec<
        'bump,
        (
            VisitedGraphs,
            HashSet<TypeAwareSelection>,
            OperationPath<'bump>,
        ),
    >,
    ctx: &'a WalkContext<'bump>,
}

impl<'bump, 'a> IndirectPathsLookupQueue<'bump, 'a> {
    pub fn new_from_excluded(
        ctx: &'a WalkContext<'bump>,
        excluded: &ExcludedFromLookup,
        path: &OperationPath<'bump>,
    ) -> Self {
        let mut queue = BumpVec::new_in(ctx.arena);
        queue.push((
            excluded.graph_ids.clone(),
            excluded.requirement.clone(),
            path.clone(),
        ));
        IndirectPathsLookupQueue { queue, ctx }
    }

    pub fn add(
        &mut self,
        visited_graphs: VisitedGraphs,
        selections: HashSet<TypeAwareSelection>,
        path: OperationPath<'bump>,
    ) {
        self.queue.push((visited_graphs, selections, path));
    }

    pub fn pop(
        &mut self,
    ) -> Option<(
        VisitedGraphs,
        HashSet<TypeAwareSelection>,
        OperationPath<'bump>,
    )> {
        self.queue.pop()
    }
}

#[derive(Debug)]
pub enum NavigationTarget<'a> {
    Field(&'a FieldSelection),
    ConcreteType(&'a str),
}

#[instrument(level = "trace",skip(ctx, path, excluded, target), fields(
  path = path.pretty_print(ctx.graph),
  current_cost = path.cost
))]
pub fn find_indirect_paths<'bump, 'a: 'bump>(
    ctx: &'a WalkContext<'bump>,
    path: &OperationPath<'bump>,
    target: &NavigationTarget,
    excluded: &ExcludedFromLookup,
) -> Result<BumpVec<'bump, OperationPath<'bump>>, WalkOperationError> {
    let mut tracker = BestPathTracker::new(ctx);
    let tail_node_index = path.tail();
    let tail_node = ctx.graph.node(tail_node_index)?;
    let source_graph_id = tail_node
        .graph_id()
        .ok_or(WalkOperationError::TailMissingInfo(tail_node_index))?;

    let mut queue = IndirectPathsLookupQueue::new_from_excluded(ctx, excluded, path);

    while let Some(item) = queue.pop() {
        let (visited_graphs, visited_key_fields, current_path) = item;

        let relevant_edges = ctx.graph.edges_from(current_path.tail()).filter(|e| {
            matches!(
                e.weight(),
                Edge::EntityMove { .. } | Edge::InterfaceObjectTypeMove { .. }
            )
        });

        for edge_ref in relevant_edges {
            trace!(
                "Exploring edge {}",
                ctx.graph.pretty_print_edge(edge_ref.id(), false)
            );

            let edge_tail_graph_id = ctx.graph.node(edge_ref.target().id())?.graph_id().unwrap();

            if visited_graphs.contains(edge_tail_graph_id) {
                trace!(
                    "Ignoring, graph is excluded and already visited (current: {}, visited: {:?})",
                    edge_tail_graph_id,
                    visited_graphs
                );
                continue;
            }

            let edge = edge_ref.weight();

            if edge_tail_graph_id == source_graph_id
                && !matches!(edge, Edge::InterfaceObjectTypeMove(..))
            {
                trace!("Ignoring. We would go back to the same graph");
                continue;
            }

            let requirements_already_checked = match edge.requirements() {
                Some(selection_requirements) => visited_key_fields.contains(selection_requirements),
                None => false,
            };

            if requirements_already_checked {
                trace!("Ignoring. Already visited similar edge");
                continue;
            }

            let new_excluded = excluded.next(edge_tail_graph_id, &visited_key_fields);

            let can_be_satisfied =
                can_satisfy_edge(ctx, edge_ref, &current_path, &new_excluded, false)?;

            if let Some(paths) = can_be_satisfied {
                trace!(
                    "Advancing path to {}",
                    ctx.graph.pretty_print_edge(edge_ref.id(), false)
                );

                let requirement_tree = QueryTreeNode::from_paths(ctx, &paths, None)?;

                let next_resolution_path = current_path.advance(
                    ctx,
                    &edge_ref,
                    requirement_tree,
                    match target {
                        NavigationTarget::Field(field) => Some(field),
                        NavigationTarget::ConcreteType(_) => None,
                    },
                );

                let direct_paths = find_direct_paths(ctx, &next_resolution_path, target)?;

                if !direct_paths.is_empty() {
                    trace!(
                        "Found {} direct paths to {}",
                        direct_paths.len(),
                        ctx.graph.pretty_print_edge(edge_ref.id(), false)
                    );

                    for direct_path in direct_paths {
                        tracker.add(&direct_path)?;
                    }
                } else {
                    trace!("No direct paths found, going deeper");

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
                }
            }
        }
    }

    let best_paths = tracker.get_best_paths();

    trace!(
        "Finished finding indirect paths, found total of {}",
        best_paths.len()
    );

    Ok(best_paths)
}

#[instrument(level = "trace",skip(ctx, path, target), fields(
    path = path.pretty_print(ctx.graph),
    current_cost = path.cost,
))]
pub fn find_direct_paths<'bump, 'a: 'bump>(
    ctx: &'a WalkContext<'bump>,
    path: &OperationPath<'bump>,
    target: &NavigationTarget,
) -> Result<BumpVec<'bump, OperationPath<'bump>>, WalkOperationError> {
    let mut result = BumpVec::new_in(ctx.arena);
    let path_tail_index = path.tail();

    match *target {
        NavigationTarget::Field(field) => {
            let edges_iter = ctx
                .graph
                .edges_from(path_tail_index)
                .filter(|e| matches!(e.weight(), Edge::FieldMove(f) if f.name == field.name));
            for edge_ref in edges_iter {
                trace!(
                    "checking edge {}",
                    ctx.graph.pretty_print_edge(edge_ref.id(), false)
                );

                let can_be_satisfied =
                    can_satisfy_edge(ctx, edge_ref, path, &ExcludedFromLookup::new(), false)?;

                if let Some(paths) = can_be_satisfied {
                    trace!(
                        "Advancing path {} with edge {}",
                        path.pretty_print(ctx.graph),
                        ctx.graph.pretty_print_edge(edge_ref.id(), false)
                    );

                    let requirement_tree = QueryTreeNode::from_paths(ctx, &paths, None)?;

                    let next_resolution_path =
                        path.advance(ctx, &edge_ref, requirement_tree, Some(field));

                    result.push(next_resolution_path);
                }
            }
        }
        NavigationTarget::ConcreteType(type_name) => {
            let edges_iter = ctx
                .graph
                .edges_from(path_tail_index)
                .filter(|e| match e.weight() {
                    Edge::AbstractMove(t) => t == type_name,
                    Edge::InterfaceObjectTypeMove(t) => t.object_type_name == type_name,
                    _ => false,
                });

            for edge_ref in edges_iter {
                trace!(
                    "Checking edge {}",
                    ctx.graph.pretty_print_edge(edge_ref.id(), false)
                );

                let can_be_satisfied =
                    can_satisfy_edge(ctx, edge_ref, path, &ExcludedFromLookup::new(), false)?;

                if let Some(paths) = can_be_satisfied {
                    trace!(
                        "Advancing path {} with edge {}",
                        path.pretty_print(ctx.graph),
                        ctx.graph.pretty_print_edge(edge_ref.id(), false)
                    );

                    let requirement_tree = QueryTreeNode::from_paths(ctx, &paths, None)?;

                    let next_resolution_path = path.advance(ctx, &edge_ref, requirement_tree, None);

                    result.push(next_resolution_path);
                }
            }
        }
    }

    trace!(
        "Finished finding direct paths, found total of {}",
        result.len()
    );

    Ok(result)
}

#[instrument(level = "trace",skip_all, fields(
  path = path.pretty_print(ctx.graph),
  edge = edge_ref.weight().display_name(),
))]
pub fn can_satisfy_edge<'bump, 'a: 'bump>(
    ctx: &'a WalkContext<'bump>,
    edge_ref: EdgeReference<'a>,
    path: &OperationPath<'bump>,
    excluded: &ExcludedFromLookup,
    use_only_direct_edges: bool,
) -> Result<Option<BumpVec<'bump, OperationPath<'bump>>>, WalkOperationError> {
    let edge = edge_ref.weight();

    match edge.requirements() {
        None => Ok(Some(BumpVec::new_in(ctx.arena))),
        Some(selections) => {
            trace!(
                "checking requirements {} for edge '{}'",
                selections,
                ctx.graph.pretty_print_edge(edge_ref.id(), false)
            );

            let mut requirements: VecDeque<MoveRequirement> = VecDeque::new();
            let mut paths_to_requirements = BumpVec::new_in(ctx.arena);

            let initial_path_slice = ctx.arena.alloc_slice_clone(&[path.clone()]);

            for selection in selections.selection_set.items.iter() {
                requirements.push_front(MoveRequirement {
                    paths: initial_path_slice,
                    selection,
                });
            }

            while let Some(requirement) = requirements.pop_back() {
                match &requirement.selection {
                    SelectionItem::Field(selection_field_requirement) => {
                        let result = validate_field_requirement(
                            ctx,
                            &requirement,
                            selection_field_requirement,
                            excluded,
                            use_only_direct_edges,
                        )?;

                        if let Some((next_paths, next_requirements)) = result {
                            if selection_field_requirement.is_leaf() {
                                let best_paths = find_best_paths(ctx.arena, next_paths);
                                for best_path in best_paths {
                                    paths_to_requirements.push(
                                        path.build_requirement_continuation_path(ctx, &best_path),
                                    );
                                }
                            }
                            requirements.extend(next_requirements);
                        } else {
                            return Ok(None);
                        }
                    }
                    SelectionItem::InlineFragment(fragment_selection) => {
                        let result = validate_fragment_requirement(
                            ctx,
                            &requirement,
                            fragment_selection,
                            excluded,
                        )?;

                        if let Some((_next_paths, next_requirements)) = result {
                            requirements.extend(next_requirements);
                        } else {
                            return Ok(None);
                        }
                    }
                    SelectionItem::FragmentSpread(_) => {}
                }
            }

            Ok(Some(paths_to_requirements))
        }
    }
}

#[derive(Debug)]
pub struct MoveRequirement<'bump, 'a> {
    pub paths: &'bump [OperationPath<'bump>],
    pub selection: &'a SelectionItem,
}

type RequirementsResult<'bump, 'a> = Option<(
    BumpVec<'bump, OperationPath<'bump>>,
    BumpVec<'bump, MoveRequirement<'bump, 'a>>,
)>;

#[instrument(level = "trace", skip_all, fields(field = field.name))]
fn validate_field_requirement<'bump, 'a: 'bump>(
    ctx: &'a WalkContext<'bump>,
    move_requirement: &MoveRequirement<'bump, 'a>,
    field: &'a FieldSelection,
    excluded: &ExcludedFromLookup,
    use_only_direct_edges: bool,
) -> Result<RequirementsResult<'bump, 'a>, WalkOperationError> {
    let mut next_paths = BumpVec::new_in(ctx.arena);

    for path in move_requirement.paths.iter() {
        let direct_paths = find_direct_paths(ctx, path, &NavigationTarget::Field(field))?;
        next_paths.extend(direct_paths);

        if !use_only_direct_edges {
            let indirect_paths =
                find_indirect_paths(ctx, path, &NavigationTarget::Field(field), excluded)?;
            next_paths.extend(indirect_paths);
        }
    }

    if next_paths.is_empty() {
        return Ok(None);
    }

    let selections = &field.selections.items;
    if selections.is_empty() {
        return Ok(Some((next_paths, BumpVec::new_in(ctx.arena))));
    }

    let shared_next_paths_for_subs = next_paths.into_bump_slice();
    let mut next_requirements = BumpVec::new_in(ctx.arena);
    next_requirements.extend(selections.iter().map(|selection_item| MoveRequirement {
        selection: selection_item,
        paths: shared_next_paths_for_subs,
    }));

    Ok(Some((
        BumpVec::from_iter_in(shared_next_paths_for_subs.iter().cloned(), ctx.arena),
        next_requirements,
    )))
}

#[instrument(level = "trace", skip_all, fields(type_condition = fragment_selection.type_condition))]
fn validate_fragment_requirement<'bump, 'a: 'bump>(
    ctx: &'a WalkContext<'bump>,
    requirement: &MoveRequirement<'bump, 'a>,
    fragment_selection: &'a InlineFragmentSelection,
    excluded: &ExcludedFromLookup,
) -> Result<RequirementsResult<'bump, 'a>, WalkOperationError> {
    let type_name = &fragment_selection.type_condition;
    let mut next_paths = BumpVec::new_in(ctx.arena);

    for path in requirement.paths.iter() {
        let direct_paths =
            find_direct_paths(ctx, path, &NavigationTarget::ConcreteType(type_name))?;
        next_paths.extend(direct_paths);
        let indirect_paths = find_indirect_paths(
            ctx,
            path,
            &NavigationTarget::ConcreteType(type_name),
            excluded,
        )?;
        next_paths.extend(indirect_paths);
    }

    if next_paths.is_empty() {
        return Ok(None);
    }

    let selections = &fragment_selection.selections.items;
    if selections.is_empty() {
        return Ok(Some((next_paths, BumpVec::new_in(ctx.arena))));
    }

    let shared_next_paths_for_subs = next_paths.into_bump_slice();
    let mut next_requirements = BumpVec::new_in(ctx.arena);
    next_requirements.extend(selections.iter().map(|selection_item| MoveRequirement {
        selection: selection_item,
        paths: shared_next_paths_for_subs,
    }));

    Ok(Some((
        BumpVec::from_iter_in(shared_next_paths_for_subs.iter().cloned(), ctx.arena),
        next_requirements,
    )))
}
