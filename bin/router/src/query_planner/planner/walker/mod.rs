mod best_path;
pub(crate) mod error;
mod excluded;
pub(crate) mod fragment_groups;
pub(crate) mod path;
pub(crate) mod pathfinder;
pub(crate) mod utils;

use std::{borrow::Cow, collections::VecDeque, rc::Rc};

use crate::query_planner::{
    ast::{
        merge_path::Condition,
        operation::OperationDefinition,
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    },
    graph::{
        edge::{Edge, PlannerOverrideContext},
        node::Node,
        Graph,
    },
    planner::walker::{
        path::UnionContext,
        pathfinder::{find_direct_path, find_self_referencing_direct_path, NavigationTarget},
    },
    state::supergraph_state::{OperationKind, SupergraphState},
    utils::cancellation::CancellationToken,
};
use best_path::{find_leaf_paths, BestPathTracker};
use error::WalkOperationError;
use excluded::ExcludedFromLookup;
use path::OperationPath;
use path::SelectionAttributes;
use pathfinder::{find_direct_paths, PathSearch, SearchCache};
use tracing::{instrument, span, trace, Level};
use utils::get_entrypoints;

pub struct ResolvedOperation<'graph> {
    pub operation_kind: OperationKind,
    pub root_field_groups: Vec<BestPathsPerLeaf<'graph>>,
    /// Root fields of a mutation or subscription that more than one subgraph can run,
    /// by their index in `root_field_groups`, with the paths from each of those subgraphs.
    /// Their spot in `root_field_groups` is left empty, the planner picks one.
    pub entrypoint_choices: Vec<(usize, Vec<BestPathsPerLeaf<'graph>>)>,
}

// TODO: Make a better struct
pub type BestPathsPerLeaf<'graph> = Vec<Vec<OperationPath<'graph>>>;

// TODO: Consider to use VecDeque(fixed_size) if we can predict it?
// TODO: Consider to drop this IR layer and just go with QTP directly.

/// A selection to plan, the paths to it, and for a fragment, the other types it's planned for,
/// see `grouped_items`.
type WorkItem<'graph, 'op> = (
    &'op SelectionItem,
    Rc<Vec<OperationPath<'graph>>>,
    Option<Rc<SelectionAttributes>>,
);
type ResolutionStack<'graph, 'op> = Vec<WorkItem<'graph, 'op>>;

/// What every step of a walk reads: the graph and what goes with it, the searches it caches, and
/// the fragments planned for more types than their own, see `fragment_groups`.
struct Walk<'graph> {
    graph: &'graph Graph<'graph>,
    supergraph: &'graph SupergraphState,
    override_context: &'graph PlannerOverrideContext,
    cancellation_token: &'graph CancellationToken,
    cache: SearchCache<'graph>,
    groups: Option<&'graph fragment_groups::FragmentGroups>,
}

impl<'graph> Walk<'graph> {
    fn find_indirect_paths(
        &self,
        path: &OperationPath<'graph>,
        target: &NavigationTarget<'graph, '_>,
        excluded: &ExcludedFromLookup<'graph>,
    ) -> Result<Vec<OperationPath<'graph>>, WalkOperationError> {
        let search = || {
            PathSearch::new(
                self.graph,
                self.supergraph,
                self.override_context,
                self.cancellation_token,
            )
            .find_indirect_paths(path, target, excluded, Some(&self.cache))
        };
        // with nothing excluded a search depends only on where it starts, so what it found from
        // the same node before is reused, see `SearchCache::indirect_paths`
        if excluded.graph_ids.is_empty() && excluded.requirement.is_empty() {
            self.cache.indirect_paths(self.graph, path, target, search)
        } else {
            search()
        }
    }
}

/// `groups` are the fragments planned for more types than their own, see `fragment_groups`.
#[instrument(level = "trace", skip_all)]
pub fn walk_operation<'graph, 'op: 'graph>(
    graph: &'graph Graph<'graph>,
    supergraph: &'graph SupergraphState,
    override_context: &'graph PlannerOverrideContext,
    operation: &'op OperationDefinition,
    cancellation_token: &'graph CancellationToken,
    groups: Option<&'graph fragment_groups::FragmentGroups>,
) -> Result<ResolvedOperation<'graph>, WalkOperationError> {
    let operation_kind = operation
        .operation_kind
        .clone()
        .unwrap_or(OperationKind::Query);
    let (op_type, selection_set) = operation.parts();
    trace!("operation is of type {:?}", op_type);

    let root_entrypoints = get_entrypoints(graph, op_type)?;
    let initial_paths: Rc<Vec<OperationPath<'graph>>> = Rc::new(
        root_entrypoints
            .iter()
            .map(|edge| OperationPath::new_entrypoint(edge))
            .collect(),
    );

    let mut paths_grouped_by_root_field: Vec<BestPathsPerLeaf> =
        Vec::with_capacity(operation.selection_set.items.len());
    let walk = Walk {
        graph,
        supergraph,
        override_context,
        cancellation_token,
        cache: SearchCache::default(),
        groups,
    };
    let walk = &walk;

    // mutations and subscriptions, see below
    let one_subgraph_per_root_field = !matches!(operation_kind, OperationKind::Query);
    let mut entrypoint_choices = Vec::new();

    // It's critical to iterate over root fiels and preserve their original order
    for selection_item in selection_set.items.iter() {
        // A mutation field has to run in one subgraph, or its side effects happen twice.
        // Same for a subscription field, two event streams can't be put together.
        // Walking from all entrypoints at once lets each leaf pick its own subgraph,
        // so these get walked from one entrypoint at a time, and the planner picks one walk.
        // The leaves the picked subgraph doesn't have come through entity calls from there.
        if !one_subgraph_per_root_field || initial_paths.len() < 2 {
            paths_grouped_by_root_field.push(walk.root_field(selection_item, &initial_paths)?);
            continue;
        }

        let mut choices = Vec::new();
        for entrypoint in initial_paths.iter() {
            match walk.root_field(selection_item, &Rc::new(vec![entrypoint.clone()])) {
                Ok(paths_per_leaf) => choices.push(paths_per_leaf),
                Err(WalkOperationError::NoPathsFound(_)) => {}
                Err(err) => return Err(err),
            }
        }

        match choices.len() {
            // Walking from all of them at once gives the error to show. If it works anyway,
            // it splits the field across subgraphs, and that's never a plan for it.
            0 => {
                walk.root_field(selection_item, &initial_paths)?;
                return Err(WalkOperationError::NoPathsFound(selection_item.to_string()));
            }
            1 => paths_grouped_by_root_field.extend(choices),
            _ => {
                entrypoint_choices.push((paths_grouped_by_root_field.len(), choices));
                paths_grouped_by_root_field.push(Vec::new());
            }
        }
    }

    Ok(ResolvedOperation {
        operation_kind,
        root_field_groups: paths_grouped_by_root_field,
        entrypoint_choices,
    })
}

impl<'graph> Walk<'graph> {
    /// The paths to every leaf of a root field, starting from `initial_paths`.
    fn root_field<'op: 'graph>(
        &self,
        selection_item: &'op SelectionItem,
        initial_paths: &Rc<Vec<OperationPath<'graph>>>,
    ) -> Result<BestPathsPerLeaf<'graph>, WalkOperationError> {
        let mut stack_to_resolve: VecDeque<WorkItem> = VecDeque::new();
        stack_to_resolve.push_back((selection_item, Rc::clone(initial_paths), None));

        let mut paths_per_leaf: Vec<Vec<OperationPath<'graph>>> = vec![];

        while let Some((selection_item, paths, also)) = stack_to_resolve.pop_front() {
            self.cancellation_token.bail_if_cancelled()?;
            let (next_stack_to_resolve, new_paths_per_leaf) =
                process_selection(self, selection_item, also, &paths, &[])?;

            paths_per_leaf.extend(new_paths_per_leaf);
            for item in next_stack_to_resolve.into_iter().rev() {
                stack_to_resolve.push_front(item);
            }
        }

        Ok(paths_per_leaf)
    }
}

/// The types planned with a fragment's own, see `SelectionAttributes::also`, that can be where
/// `path` is. A field that returns a union or an interface leads to a node per type it can be,
/// each with its own `🔮` move, and a field one subgraph narrows, like `media: [Book]` where
/// it's `[Media]` elsewhere, has the one for `Book` only there: `Movie` objects don't come from
/// it.
fn also_from(
    graph: &Graph<'_>,
    path: &OperationPath<'_>,
    attributes: Option<&Rc<SelectionAttributes>>,
) -> Option<Rc<SelectionAttributes>> {
    let attributes = attributes?;
    let also = attributes.also.as_ref()?;
    let tail = path.tail();
    // the nodes the path's last field leads to, its tail among them. A `... on Union @include`
    // around the fragment adds a selfie edge after the field, it doesn't move anywhere.
    let field_targets: Vec<petgraph::graph::NodeIndex> = path
        .segments_rev()
        .find(|segment| !matches!(graph.edge(segment.edge_index), Ok(Edge::Selfie(_))))
        .and_then(|segment| {
            let Ok(Edge::FieldMove(field)) = graph.edge(segment.edge_index) else {
                return None;
            };
            let head = graph.get_edge_head(&segment.edge_index).ok()?;
            Some(
                graph
                    .edges_from(head)
                    .filter(|edge| {
                        matches!(edge.weight(), Edge::FieldMove(other) if other.name == field.name)
                    })
                    .map(|edge| edge.target())
                    .collect(),
            )
        })
        .unwrap_or_else(|| vec![tail]);
    let reachable = |type_name: &String| {
        field_targets.iter().any(|&node| {
            graph
                .edges_from(node)
                .any(|edge| matches!(edge.weight(), Edge::AbstractMove(name) if name == type_name))
        })
    };
    if also.iter().all(reachable) {
        return Some(Rc::clone(attributes));
    }
    let also: Rc<[String]> = also
        .iter()
        .filter(|name| reachable(name))
        .cloned()
        .collect();
    (!also.is_empty()).then(|| {
        Rc::new(SelectionAttributes {
            also: Some(also),
            ..(**attributes).clone()
        })
    })
}

/// `attributes` with only the types planned together that `keep` keeps, for a path that takes
/// one of them in place of the fragment's own. `None` when none is left.
fn keep(
    attributes: &Rc<SelectionAttributes>,
    keep: impl Fn(&str) -> bool,
) -> Option<Rc<SelectionAttributes>> {
    let also: Rc<[String]> = attributes
        .also
        .iter()
        .flat_map(|names| names.iter())
        .filter(|name| keep(name))
        .cloned()
        .collect();
    (!also.is_empty()).then(|| {
        Rc::new(SelectionAttributes {
            also: Some(also),
            ..(**attributes).clone()
        })
    })
}

/// Whether `path` got to its node through a field one subgraph narrows, like `media: [Book]`
/// where it's `[Media]` elsewhere. Its node is `Book`'s, but the field's type in the operation
/// is `Media`, so `... on Book` has to stay in the fetch.
fn through_narrowed_field(
    graph: &Graph<'_>,
    supergraph: &SupergraphState,
    path: &OperationPath<'_>,
) -> bool {
    let Some(segment) = path.last_segment.as_ref() else {
        return false;
    };
    let Ok(Edge::FieldMove(field)) = graph.edge(segment.edge_index) else {
        return false;
    };
    field
        .join_field
        .and_then(|join_field| join_field.type_in_graph.as_ref())
        .is_some_and(|type_in_graph| {
            supergraph.field_return_type_name(field.type_name, field.name)
                != Some(type_in_graph.inner_type())
        })
}

/// The other types `item` is planned for, when it's a fragment planned for several.
fn also_of(walk: &Walk<'_>, item: &SelectionItem) -> Option<Rc<[String]>> {
    walk.groups?.of(item)?.also.clone()
}

fn process_selection<'graph, 'op: 'graph>(
    walk: &Walk<'graph>,
    selection_item: &'op SelectionItem,
    also: Option<Rc<SelectionAttributes>>,
    paths: &[OperationPath<'graph>],
    fields_to_resolve_locally: &[String],
) -> Result<
    (
        ResolutionStack<'graph, 'op>,
        Vec<Vec<OperationPath<'graph>>>,
    ),
    WalkOperationError,
> {
    let mut stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath<'graph>>> = vec![];

    match selection_item {
        SelectionItem::InlineFragment(fragment) => {
            let (next_selection_items, new_paths_per_leaf) =
                process_inline_fragment(walk, fragment, also, paths, fields_to_resolve_locally)?;
            paths_per_leaf.extend(new_paths_per_leaf);
            stack_to_resolve.extend(next_selection_items);
        }
        SelectionItem::Field(field) => {
            // Only fragments look at `fields_to_resolve_locally`, fields can always jump.
            let (next_selection_items, new_paths_per_leaf) = process_field(walk, field, paths)?;
            paths_per_leaf.extend(new_paths_per_leaf);
            stack_to_resolve.extend(next_selection_items);
        }
        SelectionItem::FragmentSpread(_) => {
            // No processing needed for FragmentSpread
        }
    }

    Ok((stack_to_resolve, paths_per_leaf))
}

#[instrument(level = "trace", skip_all)]
fn process_selection_set<'graph, 'op: 'graph>(
    walk: &Walk<'graph>,
    selection_set: &'op SelectionSet,
    paths: &[OperationPath<'graph>],
    fields_to_resolve_locally: &[String],
) -> Result<
    (
        ResolutionStack<'graph, 'op>,
        Vec<Vec<OperationPath<'graph>>>,
    ),
    WalkOperationError,
> {
    let mut stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath<'graph>>> = vec![];

    for item in selection_set.items.iter() {
        let also = walk.groups.and_then(|groups| groups.of(item));
        let (next_stack_to_resolve, new_paths_per_leaf) =
            process_selection(walk, item, also, paths, fields_to_resolve_locally)?;
        paths_per_leaf.extend(new_paths_per_leaf);
        stack_to_resolve.extend(next_stack_to_resolve);
    }

    Ok((stack_to_resolve, paths_per_leaf))
}

#[instrument(level = "trace", skip_all, fields(
  type_condition = fragment.type_condition,
))]
fn process_inline_fragment<'graph, 'op: 'graph>(
    walk: &Walk<'graph>,
    fragment: &'op InlineFragmentSelection,
    also: Option<Rc<SelectionAttributes>>,
    paths: &[OperationPath<'graph>],
    fields_to_resolve_locally: &[String],
) -> Result<
    (
        ResolutionStack<'graph, 'op>,
        Vec<Vec<OperationPath<'graph>>>,
    ),
    WalkOperationError,
> {
    let Walk {
        graph,
        supergraph,
        override_context,
        cancellation_token,
        ..
    } = walk;
    trace!(
        "Processing inline fragment '{}' on type '{}' (skip: {:?}, include: {:?}) through {} possible paths",
        fragment.selections,
        fragment.type_condition,
        fragment.include_if,
        fragment.skip_if,
        paths.len()
    );

    cancellation_token.bail_if_cancelled()?;

    // if the current type is an object type we ignore an abstract move
    // but if it's a union, we need to find an abstract move to the target type
    let tail_index = graph.get_edge_tail(
        &paths
            .first()
            .unwrap()
            .last_segment
            .as_ref()
            .unwrap()
            .edge_index,
    )?;

    // a fragment on the tail's own type always applies, so there's no `... on X` to look for
    //  - interfaces - `... on Interface` - we look for the interface's fields.
    //  - object types - `... on Object` - we look for the object's fields.
    //  - union types - `... on Union` on a field that returns the union. The tail is one of the
    //    union's member nodes, named after the union, and the path stays on that member.
    // without a condition we go straight to the fragment's selections, with one the fragment is
    // kept through a selfie edge, every one of these nodes has one
    let tail = graph.node(tail_index)?;
    let tail_type_name = match tail {
        Node::SubgraphType(t) => &t.name,
        _ => panic!("Expected a subgraph type when resolving fragments"),
    };

    // whether a path is at the fragment's type already: only the first path's tail was checked
    // above. One at an interface object that got to the type with `🔎 Type` has no selfie edge,
    // its tail is still the interface object, so it takes that type condition again.
    let at_the_type = |path: &OperationPath<'graph>| -> Result<bool, WalkOperationError> {
        Ok(matches!(
            graph.node(path.tail())?,
            Node::SubgraphType(t) if t.name == fragment.type_condition
        ))
    };
    // the fragment's type or one planned with it, see `SelectionAttributes::also`, when a path
    // is at it already. Like `Book` under a `media` that one subgraph has as `[Book]`.
    let at_a_planned_type =
        |path: &OperationPath<'graph>| -> Result<Option<&'graph str>, WalkOperationError> {
            let Node::SubgraphType(t) = graph.node(path.tail())? else {
                return Ok(None);
            };
            let planned = t.name == fragment.type_condition
                || also
                    .as_ref()
                    .and_then(|attributes| attributes.also.as_ref())
                    .is_some_and(|names| names.iter().any(|name| name == t.name));
            Ok(planned.then_some(t.name))
        };
    let condition: Option<Condition> = fragment.into();
    // A path at the type has nothing to look for: without a condition it goes straight to the
    // fragment's selections, with one it keeps the fragment through a selfie edge. So does one
    // that got there through a narrowed field: the field's type is still the interface. Its
    // objects are all of the type, so the other types planned with it aren't there.
    let stay_at = |path: &OperationPath<'graph>,
                   type_name: &'graph str|
     -> Result<OperationPath<'graph>, WalkOperationError> {
        if condition.is_none() && !through_narrowed_field(graph, supergraph, path) {
            return Ok(path.clone());
        }
        find_self_referencing_direct_path(
            graph,
            supergraph,
            override_context,
            path,
            type_name,
            condition.as_ref(),
            cancellation_token,
        )
    };

    if tail_type_name == &fragment.type_condition {
        // It's the same type and no conditions are applied, we can skip the fragment processing
        // and go directly to its selections.
        if condition.is_none()
            && paths.iter().skip(1).try_fold(true, |all, path| {
                Ok::<_, WalkOperationError>(all && at_the_type(path)?)
            })?
            && !paths
                .iter()
                .any(|path| through_narrowed_field(graph, supergraph, path))
        {
            return process_selection_set(
                walk,
                &fragment.selections,
                paths,
                fields_to_resolve_locally,
            );
        }

        // Otherwise each path on its own: one at the type stays there, any other takes the type
        // condition, with the types planned with it.
        let mut next_paths: Vec<OperationPath<'graph>> = Vec::with_capacity(paths.len());
        for path in paths {
            let path_span = span!(
                Level::TRACE,
                "explore_path",
                path = path.pretty_print(graph)
            );
            let _enter = path_span.enter();

            let direct_path = if at_the_type(path)? {
                Some(stay_at(path, &fragment.type_condition)?)
            } else {
                find_direct_path(
                    graph,
                    supergraph,
                    override_context,
                    path,
                    &NavigationTarget::ConcreteType(
                        &fragment.type_condition,
                        condition.clone(),
                        also.clone(),
                    ),
                    cancellation_token,
                )?
            };

            if let Some(direct_path) = direct_path {
                trace!("advanced: {}", path.pretty_print(graph));
                next_paths.push(direct_path);
            }
        }

        if next_paths.is_empty() {
            return Err(WalkOperationError::NoPathsFound(
                fragment.type_condition.to_string(),
            ));
        }

        // Now process the selections under the fragment using the advanced paths
        return process_selection_set(
            walk,
            &fragment.selections,
            &next_paths,
            fields_to_resolve_locally,
        );
    }

    trace!(
        "Trying to advance to: ... on {}, through {} possible paths",
        fragment.type_condition,
        paths.len()
    );

    // Takes `... on type_name` from `path`, with `also` planned with it. False when no path
    // leads there.
    let advance = |next_paths: &mut Vec<OperationPath<'graph>>,
                   path: &OperationPath<'graph>,
                   type_name: &'graph str,
                   also: Option<Rc<SelectionAttributes>>|
     -> Result<bool, WalkOperationError> {
        let target = NavigationTarget::ConcreteType(type_name, fragment.into(), also);
        let direct_path = find_direct_path(
            graph,
            supergraph,
            override_context,
            path,
            &target,
            cancellation_token,
        )?;

        let found_direct_path = direct_path.is_some();
        trace!("Direct path to {} found: {}", type_name, found_direct_path);
        if let Some(direct_path) = direct_path {
            trace!("advanced: {}", path.pretty_print(graph));
            next_paths.push(direct_path);
        }

        let mut found_indirect_paths = false;
        if fields_to_resolve_locally.is_empty() {
            let indirect_paths =
                walk.find_indirect_paths(path, &target, &ExcludedFromLookup::new())?;

            found_indirect_paths = !indirect_paths.is_empty();
            if found_indirect_paths {
                trace!("advanced: {}", path.pretty_print(graph));
                // One best path per subgraph. Keep them all: the fields under the fragment
                // may only exist in one of them (the first is not always the right one).
                next_paths.extend(indirect_paths);
            }
        }

        Ok(found_direct_path || found_indirect_paths)
    };
    // the names of the types planned with this one, as the supergraph has them, so they outlive
    // the fragment's own
    let names_of = |attributes: Option<&Rc<SelectionAttributes>>| -> Vec<&'graph str> {
        attributes
            .and_then(|attributes| attributes.also.as_deref())
            .into_iter()
            .flatten()
            .filter_map(|name| supergraph.definitions.get_key_value(name))
            .map(|(name, _)| name.as_str())
            .collect()
    };

    let mut next_paths: Vec<OperationPath<'graph>> = Vec::with_capacity(paths.len());
    // the types planned with this one that a path got to, and the paths that got nowhere
    let mut covered: Vec<&'graph str> = vec![];
    let mut missed: Vec<&OperationPath<'graph>> = vec![];
    for path in paths {
        let path_span = span!(
            Level::TRACE,
            "explore_path",
            path = path.pretty_print(graph)
        );
        let _enter = path_span.enter();

        // the types planned with this one aren't where its objects are all of it already
        if also.is_some() && at_the_type(path)? {
            next_paths.push(stay_at(path, &fragment.type_condition)?);
            continue;
        }

        // At a type planned with it through a field narrowed in that subgraph, like `Book`
        // under a `media` one subgraph has as `[Book]`, its objects can only be that type: it
        // doesn't leave out any type the other paths take, the others aren't there.
        if also.is_some() && through_narrowed_field(graph, supergraph, path) {
            if let Some(type_name) = at_a_planned_type(path)? {
                next_paths.push(stay_at(path, type_name)?);
                covered.push(type_name);
                continue;
            }
        }

        let path_also = also_from(graph, path, also.as_ref());
        if advance(
            &mut next_paths,
            path,
            &fragment.type_condition,
            path_also.clone(),
        )? {
            covered.extend(names_of(path_also.as_ref()));
        } else {
            if fields_to_resolve_locally.is_empty() {
                // Looks like a union member or an interface implementation is not resolvable.
                // The fact the fragment for that object type passed GraphQL validations,
                // means that it's a child of the abstract type,
                // and it was probably eliminated from the Graph because of intersection.
                trace!(
                    "Object type '{}' is not resolvable by '{}', resolve only the __typename",
                    fragment.type_condition,
                    tail_type_name
                );
            }
            missed.push(path);
        }
    }

    // Any other path that can't take the fragment's type may still lead to a type planned with
    // it, one no other path got to: its fields were moved into this fragment. Only then: paths
    // into the same field are alternatives, and one with fewer types could win over one with
    // them all.
    let uncovered: Vec<&'graph str> = names_of(also.as_ref())
        .into_iter()
        .filter(|name| !covered.contains(name))
        .collect();
    for path in missed.into_iter().filter(|_| !uncovered.is_empty()) {
        let path_span = span!(
            Level::TRACE,
            "explore_path",
            path = path.pretty_print(graph)
        );
        let _enter = path_span.enter();

        // one at such a type already stays there
        if let Some(type_name) = at_a_planned_type(path)? {
            if uncovered.contains(&type_name) {
                next_paths.push(stay_at(path, type_name)?);
                continue;
            }
        }

        // only a type the path's node moves to: a search for one it doesn't is most of the
        // planning time on wide abstract types
        let tail = path.tail();
        let mut candidates = uncovered
            .iter()
            .copied()
            .filter(|type_name| {
                graph.edges_from(tail).any(
                    |edge| matches!(edge.weight(), Edge::AbstractMove(name) if name == type_name),
                )
            })
            .peekable();
        let path_also = if candidates.peek().is_some() {
            also_from(graph, path, also.as_ref())
        } else {
            None
        };
        for type_name in candidates {
            let rest = path_also.as_ref().and_then(|attributes| {
                keep(attributes, |name| {
                    name != type_name && uncovered.contains(&name)
                })
            });
            if advance(&mut next_paths, path, type_name, rest)? {
                break;
            }
        }
    }

    if next_paths.is_empty() {
        let mut tracker = BestPathTracker::new(graph);

        for path in paths {
            let path_span = span!(
                Level::TRACE,
                "explore_path",
                path = path.pretty_print(graph)
            );
            let _enter = path_span.enter();
            let direct_paths = find_direct_paths(
                graph,
                supergraph,
                override_context,
                path,
                &NavigationTarget::field(&FieldSelection::new_typename(), None),
                cancellation_token,
            )?;

            trace!("Direct paths found: {}", direct_paths.len());

            if !direct_paths.is_empty() {
                for p in direct_paths {
                    tracker.add(&p)?;
                }
            } else {
                return Err(WalkOperationError::NoPathsFound("__typename".to_string()));
            }
        }

        let next_paths = tracker.get_best_paths();

        if next_paths.is_empty() {
            return Err(WalkOperationError::NoPathsFound("__typename".to_string()));
        }

        return Ok((vec![], vec![find_leaf_paths(next_paths)]));
    }

    process_selection_set(
        walk,
        &fragment.selections,
        &next_paths,
        fields_to_resolve_locally,
    )
}

/// The same union may have different members in different subgraphs.
/// For example:
///
/// - graph A: Action = Common | OnlyA
/// - graph B: Action = Common | OnlyB
///
/// If the planner still has valid candidate paths through both A and B,
/// the response shape must not depend on which path wins later.
/// In that case we keep only members that are available in every candidate graph.
///
/// In the example above, only `Common` is safe to keep.
fn narrow_partial_union_paths<'graph>(paths: &mut Vec<OperationPath<'graph>>) {
    if paths.len() <= 1 {
        return;
    }

    let Some(union_context) = paths.first().and_then(|path| path.union_context.as_ref()) else {
        return;
    };

    let all_same_field = paths.iter().all(|path| {
        path.union_context
            .as_ref()
            .is_some_and(|ctx| ctx.eq_field(union_context))
    });

    if !all_same_field {
        return;
    }

    // Collect the union members that each subgraph can return.
    //
    // All paths through the same graph share the same `possible_members` because they originate
    // from the same `UnionMembersData` (one per graph). We only need to record each graph's member
    // set once — no accumulation is required.
    let mut members_per_graph: Vec<&UnionContext<'graph>> = Vec::new();

    for path in paths.iter() {
        let Some(context) = path.union_context.as_ref() else {
            return;
        };

        if !context.eq_field(union_context) {
            return;
        }

        if !members_per_graph
            .iter()
            .any(|existing_context| existing_context.graph_id == context.graph_id)
        {
            members_per_graph.push(context);
        }
    }

    if members_per_graph.len() <= 1 {
        return;
    }

    let least_members_context = members_per_graph
        .iter()
        .min_by_key(|context| context.possible_members_len())
        // It's safe as we checked that `members_per_graph` has at least two entries above
        .expect("members_per_graph has at least two entries");

    // Start from the smallest member list.
    // Then shrink it with each graph until only shared members remain.
    let mut shared_members = least_members_context.possible_members();

    for context in &members_per_graph {
        shared_members.retain(|member| context.can_resolve_member(member));

        if shared_members.is_empty() {
            // No union member exists in every candidate subgraph.
            // None of these paths is safe to keep.
            paths.clear();
            return;
        }
    }

    paths.retain_mut(|path| {
        // It's safe to unwrap as we checked that `union_context` exists already
        let context = path
            .union_context
            .as_mut()
            .expect("union context should exist at this point");

        // Keep only paths whose current member is shared
        if !shared_members.contains(&context.member_name) {
            return false;
        }

        // This path already represents one member, so keep only that member.
        context.set_possible_member(context.member_name);
        true
    });
}

#[instrument(level = "trace", skip_all, fields(
  field_name = &field.name,
  leaf = field.is_leaf()
))]
fn process_field<'graph, 'op: 'graph>(
    walk: &Walk<'graph>,
    field: &'op FieldSelection,
    paths: &[OperationPath<'graph>],
) -> Result<
    (
        ResolutionStack<'graph, 'op>,
        Vec<Vec<OperationPath<'graph>>>,
    ),
    WalkOperationError,
> {
    let Walk {
        graph,
        supergraph,
        override_context,
        cancellation_token,
        ..
    } = walk;
    let mut next_stack_to_resolve: ResolutionStack = vec![];
    let mut paths_per_leaf: Vec<Vec<OperationPath<'graph>>> = vec![];
    let mut tracker = BestPathTracker::new(graph);

    trace!(
        "Trying to advance to: {} through {} possible paths",
        field,
        paths.len()
    );

    cancellation_token.bail_if_cancelled()?;

    let mut target_subgraph_ids_cache = None;
    // built once, so every path into the field shares its alias and arguments
    let direct_target = NavigationTarget::field(field, None);

    for path in paths {
        let path_span = span!(
            Level::TRACE,
            "explore_path",
            path = path.pretty_print(graph)
        );
        let _enter = path_span.enter();

        let mut advanced = false;

        let excluded = ExcludedFromLookup::new();
        let direct_paths = find_direct_paths(
            graph,
            supergraph,
            override_context,
            path,
            &direct_target,
            cancellation_token,
        )?;
        trace!("Direct paths found: {}", direct_paths.len());

        // A leaf reached without leaving the subgraph can't be beaten by going elsewhere, unless
        // it needs `@requires`: that's another fetch and wave, and a subgraph that has the field
        // without requirements may be one hop away.
        let found_direct_paths_to_leaf = field.is_leaf()
            && direct_paths.iter().any(|direct_path| {
                direct_path
                    .last_segment
                    .as_ref()
                    .is_none_or(|segment| segment.requirement_tree.is_none())
            });

        if !direct_paths.is_empty() {
            advanced = true;
            for direct_path in &direct_paths {
                tracker.add(direct_path)?;
            }
        }

        if !found_direct_paths_to_leaf {
            let has_indirect_candidates = graph.entity_moves_from(path.tail()).next().is_some();

            if has_indirect_candidates {
                if target_subgraph_ids_cache.is_none() {
                    target_subgraph_ids_cache =
                        Some(field_target_subgraph_ids(supergraph, field, paths, graph)?);
                }
                let target_subgraph_ids = target_subgraph_ids_cache
                    .as_ref()
                    .and_then(|ids| ids.as_deref());

                let target = direct_target.in_subgraphs(target_subgraph_ids);
                let indirect_paths = walk.find_indirect_paths(path, &target, &excluded)?;
                trace!("Indirect paths found: {}", indirect_paths.len());

                if !indirect_paths.is_empty() {
                    advanced = true;
                    for indirect_path in indirect_paths {
                        tracker.add(&indirect_path)?;
                    }
                }
            } else {
                trace!(
                    "Skipping indirect lookup for '{}' at tail {} because there are no entity/interface-object outgoing edges",
                    field.name,
                    path.tail().index()
                );
            }
        }

        trace!(
            "{}: {}",
            if advanced {
                "advanced"
            } else {
                "failed to advance"
            },
            path.pretty_print(graph)
        );
    }

    let mut next_paths = tracker.get_best_paths();
    narrow_partial_union_paths(&mut next_paths);
    if next_paths.is_empty() {
        return Err(WalkOperationError::NoPathsFound(field.name.to_string()));
    }

    let mut fields_to_resolve_locally: Vec<String> = Vec::new();
    if !field.is_leaf() {
        let field_move_paths: Vec<_> = next_paths
            .iter()
            .filter(|path| {
                path.last_segment.as_ref().is_some_and(|seg| {
                    matches!(graph.edge(seg.edge_index).unwrap(), Edge::FieldMove(_))
                })
            })
            .collect();

        if !field_move_paths.is_empty() {
            let edge_index = field_move_paths[0]
                .last_segment
                .as_ref()
                .unwrap()
                .edge_index;

            let head_index = graph.get_edge_head(&edge_index)?;
            let tail_index = graph.get_edge_tail(&edge_index)?;

            let head = graph.node(head_index)?;
            let tail = graph.node(tail_index)?;

            let parent_type_name = head.name_str();
            let parent_def = supergraph
                .definitions
                .get(parent_type_name)
                .ok_or_else(|| WalkOperationError::TypeNotFound(parent_type_name.to_string()))?;

            let field_def = parent_def.fields().get(&field.name).ok_or_else(|| {
                WalkOperationError::FieldNotFound(
                    field.name.to_string(),
                    parent_type_name.to_string(),
                )
            })?;

            let output_type = supergraph
                .definitions
                .get(tail.name_str())
                .ok_or_else(|| WalkOperationError::TypeNotFound(tail.name_str().to_string()))?;

            if output_type.is_interface_type()
                && field_def.is_shared
                // if there's one type, the query planner can decide which subgraph to use, based
                // on the fragment's type condition. A fragment planned for several types stands
                // for all of them.
                && field
                    .selections
                    .items
                    .iter()
                    .filter(|item| matches!(item, SelectionItem::InlineFragment(_)))
                    .map(|item| 1 + also_of(walk, item).map_or(0, |also| also.len()))
                    .sum::<usize>()
                    > 1
            {
                fields_to_resolve_locally = output_type
                    .fields()
                    .keys()
                    .map(|name| name.to_string())
                    .collect();
            }
        }
    }

    if !fields_to_resolve_locally.is_empty() {
        let path_span = span!(
            Level::TRACE,
            "Shareable interface detected. Validating that sub-selections can be resolved from a single path."
        );
        let _enter = path_span.enter();
        let mut valid_paths_for_children: Vec<OperationPath<'graph>> =
            Vec::with_capacity(next_paths.len());
        for candidate_path in &next_paths {
            let mut all_children_resolvable = true;
            // We don't need the results of the sub-walk here, only whether it was successful
            for child_selection in &field.selections.items {
                let finding = process_selection(
                    walk,
                    child_selection,
                    walk.groups.and_then(|groups| groups.of(child_selection)),
                    std::slice::from_ref(candidate_path),
                    &fields_to_resolve_locally,
                );

                match finding {
                    Ok((child_stack, child_leaves)) => {
                        if child_leaves.is_empty() && child_stack.is_empty() {
                            all_children_resolvable = false;
                            trace!(
                                "Path {} failed to resolve child '{}' locally.",
                                candidate_path.pretty_print(graph),
                                child_selection
                            );
                            break; // This candidate_path is invalid.
                        }
                    }
                    Err(_) => {
                        all_children_resolvable = false;
                        break; // This candidate_path is invalid.
                    }
                }
            }

            if all_children_resolvable {
                trace!(
                    "Path {} can resolve all children locally and is valid.",
                    candidate_path.pretty_print(graph)
                );
                valid_paths_for_children.push(candidate_path.clone());
            }
        }
        next_paths = valid_paths_for_children;
        if !field.is_leaf() && next_paths.is_empty() {
            // If no single path could satisfy all children, we have no valid way forward.
            return Err(WalkOperationError::NoPathsFound(field.name.to_string()));
        }
    }

    if field.is_leaf() {
        paths_per_leaf.push(find_leaf_paths(next_paths));
    } else {
        trace!("Found {} paths", next_paths.len());
        let next_paths = Rc::new(next_paths);
        for next_selection_items in field.selections.items.iter() {
            let also = walk
                .groups
                .and_then(|groups| groups.of(next_selection_items));
            next_stack_to_resolve.push((next_selection_items, Rc::clone(&next_paths), also));
        }
    }

    Ok((next_stack_to_resolve, paths_per_leaf))
}

/// The subgraphs that can resolve `field` on the types the paths end at, as sorted indices
/// into `SupergraphState::subgraph_names`. Borrowed from the field when that's one type,
/// merged when it's several.
fn field_target_subgraph_ids<'s>(
    supergraph: &'s SupergraphState,
    field: &FieldSelection,
    paths: &[OperationPath<'_>],
    graph: &Graph<'_>,
) -> Result<Option<Cow<'s, [u16]>>, WalkOperationError> {
    let mut target_subgraph_ids: Option<Cow<'s, [u16]>> = None;

    for path in paths {
        let parent_type_name = graph.node(path.tail())?.name_str();
        let Some(field_def) = supergraph
            .definitions
            .get(parent_type_name)
            .and_then(|parent_def| parent_def.fields().get(&field.name))
        else {
            continue;
        };
        let field_ids = &field_def.resolvable_in[..];

        target_subgraph_ids = Some(match target_subgraph_ids {
            None => Cow::Borrowed(field_ids),
            // Paths mostly end at the same type, so there's nothing new to add.
            Some(ids) if field_ids.iter().all(|id| ids.contains(id)) => ids,
            Some(ids) => {
                let mut merged = ids.into_owned();
                merged.extend_from_slice(field_ids);
                merged.sort_unstable();
                merged.dedup();
                Cow::Owned(merged)
            }
        });
    }

    Ok(target_subgraph_ids.filter(|ids| !ids.is_empty()))
}
