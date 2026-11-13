/// If a query asks for the same fields on object types,
/// and those objects all belong to the same interface (based on the field's return type),
/// we can replace inline fragments for each object type with a single interface fragment.
///
/// During normalization, under a certain condition, the requested interface fields
/// are replaced with inline fragments for each object type that implements the interface.
///
/// In some cases we can fold multiple inline fragments into a single one, for a given interface.
/// This is what this file does.
///
/// Example:
///
///   query {
///     media {
///       ... on Book { id title }
///       ... on Movie { id title }
///     }
///   }
///
/// ->
///
///   query {
///     media { id title }
///   }
///
use std::collections::{BTreeMap, BTreeSet};

use tracing::instrument;

use crate::query_planner::{
    ast::{
        flat::{
            arena::{Arena, ListId, NodeId},
            Kind,
        },
        merge_path::{Condition, MergePath, Segment},
    },
    planner::fetch::{
        error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
        selections::FetchStepSelections, state::MultiTypeFetchStep,
    },
    planner::QueryPlannerOptions,
    state::{
        subgraph_state::{SubgraphField, SubgraphState},
        supergraph_state::{OperationKind, SupergraphDefinition, SupergraphState},
    },
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn fold_concrete_selections_to_interfaces(
        &mut self,
        supergraph: &SupergraphState,
        options: &QueryPlannerOptions,
    ) -> Result<bool, FetchGraphError> {
        let root_type_name = match self.operation_kind {
            OperationKind::Query => supergraph.query_type.as_str(),
            OperationKind::Mutation => supergraph.mutation_type.as_deref().ok_or_else(|| {
                FetchGraphError::Internal("Expected mutation type to exist".to_string())
            })?,
            OperationKind::Subscription => {
                supergraph.subscription_type.as_deref().ok_or_else(|| {
                    FetchGraphError::Internal("Expected subscription type to exist".to_string())
                })?
            }
        };

        let mut changed = false;
        let step_indices = self.step_indices().collect::<Vec<_>>();

        for index in step_indices {
            let (step, arena) = self.step_and_arena_mut(index)?;
            let Ok(subgraph) = supergraph.subgraph_state(&step.service_name) else {
                continue;
            };

            changed |= StepConverter {
                root_type_name,
                supergraph,
                subgraph,
                options,
            }
            .rewrite_step(arena, step)?;
        }

        Ok(changed)
    }
}

struct StepConverter<'a> {
    root_type_name: &'a str,
    supergraph: &'a SupergraphState,
    subgraph: &'a SubgraphState,
    options: &'a QueryPlannerOptions,
}

struct InterfaceInSubgraph<'a> {
    fields: &'a [SubgraphField],
    members: BTreeSet<&'a str>,
}

/// Object-type branch that we might try to fold into an interface
struct ObjectTypeBranch<'a> {
    type_name: &'a str,
    selections: ListId,
    condition: Option<Condition>,
    remove_index: Option<usize>,
}

/// The result of folding concrete object type into one interface
struct FoldedSelection {
    /// The first branch's selections. Its branch goes away, so they can move.
    selections: ListId,
    condition: Option<Condition>,
    remove_indexes: Vec<usize>,
}

struct FoldCandidate<'a> {
    interface_name: &'a str,
    folded: FoldedSelection,
}

impl<'a> StepConverter<'a> {
    /// Changes the step's output lists in place, they're its own.
    fn rewrite_step(
        &self,
        arena: &mut Arena,
        step: &mut FetchStepData<MultiTypeFetchStep>,
    ) -> Result<bool, FetchGraphError> {
        let mut output: BTreeMap<String, ListId> = step
            .output
            .iter()
            .map(|(type_name, list)| (type_name.clone(), list))
            .collect();
        let mut changed = false;
        changed |= self.rewrite_root_output(arena, &step.response_path, &mut output)?;
        changed |= self.rewrite_nested_output(arena, &output)?;
        if changed {
            step.output = FetchStepSelections::from_lists(output);
        }
        Ok(changed)
    }

    /// Try to turn top-level object-type selections into an interface selection.
    ///
    /// When `Book` and `Movie` both implement `Media`, we turn
    ///
    ///   `{ Book: { id } Movie: { id } }`
    ///
    /// into
    ///
    ///   `{ Media: { id } }`
    fn rewrite_root_output(
        &self,
        arena: &Arena,
        response_path: &MergePath,
        output: &mut BTreeMap<String, ListId>,
    ) -> Result<bool, FetchGraphError> {
        // Only rewrite if the step is at the root (no response path).
        if !response_path.inner.is_empty() {
            return Ok(false);
        }

        let interface_name =
            response_path.resolve_type_name(self.root_type_name, self.supergraph)?;

        let branches = output
            .iter()
            .map(|(type_name, &selections)| ObjectTypeBranch {
                type_name: type_name.as_str(),
                selections,
                condition: None,
                remove_index: None,
            })
            .collect::<Vec<_>>();

        let Some(candidate) = self.try_fold(arena, interface_name, &branches)? else {
            return Ok(false);
        };

        output.clear();
        output.insert(
            candidate.interface_name.to_string(),
            candidate.folded.selections,
        );

        Ok(true)
    }

    /// Before:
    ///
    /// `{ media { ... on Book { id } ... on Movie { id } } }`
    ///
    /// After:
    ///
    /// `{ media { id } }`
    fn rewrite_nested_output(
        &self,
        arena: &mut Arena,
        output: &BTreeMap<String, ListId>,
    ) -> Result<bool, FetchGraphError> {
        let mut changed = false;

        for (definition_name, &list) in output.iter() {
            changed |= self.rewrite_selection_set(arena, definition_name, list)?;
        }

        Ok(changed)
    }

    fn rewrite_selection_set(
        &self,
        arena: &mut Arena,
        current_type_name: &str,
        list: ListId,
    ) -> Result<bool, FetchGraphError> {
        let mut changed = false;

        for index in 0..arena.items(list).len() {
            let node = *arena.node(arena.items(list)[index]);
            match node.kind {
                Kind::Field => {
                    let child_type_name = self
                        .supergraph
                        .field_return_type_name(current_type_name, arena.str(node.name))
                        .ok_or_else(|| {
                            FetchGraphError::Internal(format!(
                                "No field found for name '{}' in type '{}'",
                                arena.str(node.name),
                                current_type_name
                            ))
                        })?;
                    changed |= self.rewrite_selection_set(arena, child_type_name, node.children)?;
                }
                Kind::InlineFragment => {
                    let type_condition = arena.str(node.name).to_string();
                    changed |= self.rewrite_selection_set(arena, &type_condition, node.children)?;
                }
                Kind::FragmentSpread => {
                    // Fragment spreads should have been inlined before we get here.
                    // If we see one, something went wrong earlier.
                    return Err(FetchGraphError::Internal(
                        "fragment spreads should have been inlined before abstract type conversion"
                            .to_string(),
                    ));
                }
            }
        }
        changed |= self.rewrite_inline_fragments(arena, current_type_name, list)?;

        Ok(changed)
    }

    fn rewrite_inline_fragments(
        &self,
        arena: &mut Arena,
        current_type_name: &str,
        list: ListId,
    ) -> Result<bool, FetchGraphError> {
        let candidate = {
            let arena = &*arena;
            let branches = arena
                .items(list)
                .iter()
                .enumerate()
                .filter_map(|(index, &id)| {
                    let node = arena.node(id);
                    (node.kind == Kind::InlineFragment).then(|| ObjectTypeBranch {
                        type_name: arena.str(node.name),
                        selections: node.children,
                        condition: arena.condition(id),
                        remove_index: Some(index),
                    })
                })
                .collect::<Vec<_>>();

            self.try_fold(arena, current_type_name, &branches)?
        };

        let Some(candidate) = candidate else {
            return Ok(false);
        };

        // Remove the branches we folded
        let kept: Vec<NodeId> = arena
            .items(list)
            .iter()
            .enumerate()
            .filter(|(index, _)| !candidate.folded.remove_indexes.contains(index))
            .map(|(_, &id)| id)
            .collect();
        arena.set_items(list, &kept);

        // With @skip/@include on the branches, or folded into a type other than the parent's,
        // they go under one `... on Interface`. Otherwise right into the parent.
        if candidate.folded.condition.is_some() || candidate.interface_name != current_type_name {
            let fragment = arena.new_fragment(
                candidate.interface_name,
                candidate.folded.condition.as_ref(),
            );
            arena.node_mut(fragment).children = candidate.folded.selections;
            arena.push(list, fragment);
        } else {
            arena.merge(list, candidate.folded.selections, false)?;
        }

        Ok(true)
    }

    fn try_fold<'b>(
        &'a self,
        arena: &Arena,
        default_interface_name: &'b str,
        branches: &[ObjectTypeBranch<'_>],
    ) -> Result<Option<FoldCandidate<'b>>, FetchGraphError>
    where
        'a: 'b,
    {
        if self.options.experimental_abstract_type_folding {
            self.try_fold_into_any_matching_interface(arena, branches)
        } else {
            self.try_fold_into_interface(arena, default_interface_name, branches)
        }
    }

    fn try_fold_into_any_matching_interface<'b>(
        &'a self,
        arena: &Arena,
        branches: &[ObjectTypeBranch<'_>],
    ) -> Result<Option<FoldCandidate<'b>>, FetchGraphError>
    where
        'a: 'b,
    {
        let Some(first_branch) = branches.first() else {
            return Ok(None);
        };

        let Some(SupergraphDefinition::Object(object_type)) =
            self.supergraph.definitions.get(first_branch.type_name)
        else {
            return Ok(None);
        };

        let mut interface_names = object_type
            .join_implements
            .iter()
            .filter(|join_implements| join_implements.graph_id == self.subgraph.graph_id)
            .map(|join_implements| join_implements.interface.as_str())
            .collect::<Vec<_>>();

        interface_names.sort_unstable();
        interface_names.dedup();

        for interface_name in interface_names {
            if let Some(candidate) =
                self.try_fold_into_interface(arena, interface_name, branches)?
            {
                return Ok(Some(candidate));
            }
        }

        Ok(None)
    }

    fn try_fold_into_interface<'b>(
        &'a self,
        arena: &Arena,
        interface_name: &'b str,
        branches: &[ObjectTypeBranch<'_>],
    ) -> Result<Option<FoldCandidate<'b>>, FetchGraphError>
    where
        'a: 'b,
    {
        // Folding 1 branch is pointless
        if branches.len() < 2 {
            return Ok(None);
        }

        // The interface exists in this subgraph
        let Some(interface) = self.local_interface(interface_name) else {
            return Ok(None);
        };

        let Some(first_branch) = branches.first() else {
            return Ok(None);
        };

        let shared_condition = first_branch.condition.clone();
        let mut concrete_types = BTreeSet::new();

        for branch in branches {
            // Every branch condition (@skip / @include) must be the same
            if branch.condition != shared_condition {
                return Ok(None);
            }

            if !self.object_type_exists_in_subgraph(branch.type_name) {
                // The object type does not exist in this subgraph
                return Ok(None);
            }

            concrete_types.insert(branch.type_name);
        }

        // The types must exactly match all interface members in this subgraph
        if concrete_types != interface.members {
            return Ok(None);
        }

        let selections = first_branch.selections;

        if !branches
            .iter()
            .skip(1)
            // All must branches ask for the same fields
            .all(|branch| arena.same_selections(selections, branch.selections))
        {
            return Ok(None);
        }

        if !self.selected_fields_exist_on_interface(
            arena,
            first_branch.type_name,
            selections,
            interface.fields,
        ) {
            // The selected fields do not exist on the interface
            return Ok(None);
        }

        if !self.selection_set_is_valid_for_type(arena, interface_name, selections)? {
            // The selection set (fields and fragments) is not valid for the interface
            return Ok(None);
        }

        Ok(Some(FoldCandidate {
            interface_name,
            folded: FoldedSelection {
                selections,
                condition: shared_condition,
                remove_indexes: branches
                    .iter()
                    .filter_map(|branch| branch.remove_index)
                    .collect(),
            },
        }))
    }

    /// Check that every field exists on the interface.
    /// If a field exists on the object type but not on the interface, we cannot fold.
    /// Fields in `... on <type_name>` count too.
    fn selected_fields_exist_on_interface(
        &self,
        arena: &Arena,
        type_name: &str,
        list: ListId,
        interface_fields: &[SubgraphField],
    ) -> bool {
        arena.items(list).iter().all(|&id| {
            let node = arena.node(id);
            let name = arena.str(node.name);
            match node.kind {
                Kind::Field => {
                    name == "__typename"
                        || interface_fields
                            .iter()
                            .any(|interface_field| interface_field.name == name)
                }
                Kind::InlineFragment if name == type_name => self
                    .selected_fields_exist_on_interface(
                        arena,
                        type_name,
                        node.children,
                        interface_fields,
                    ),
                _ => true,
            }
        })
    }

    /// Check that a selection set is valid for a given type in this subgraph.
    fn selection_set_is_valid_for_type(
        &self,
        arena: &Arena,
        type_name: &str,
        list: ListId,
    ) -> Result<bool, FetchGraphError> {
        if arena.is_empty(list) {
            return Ok(true);
        }

        let Some(definition) = self.subgraph.definitions.get(type_name) else {
            return Ok(false);
        };

        for &id in arena.items(list) {
            let node = arena.node(id);
            let name = arena.str(node.name);
            match node.kind {
                Kind::Field => {
                    if name == "__typename" {
                        continue;
                    }

                    let Some(field_definition) = definition.field(name) else {
                        return Ok(false);
                    };

                    if !self.selection_set_is_valid_for_type(
                        arena,
                        field_definition.field_type.inner_type(),
                        node.children,
                    )? {
                        return Ok(false);
                    }
                }
                Kind::InlineFragment => {
                    if !self.fragment_type_is_valid_for_parent_type(type_name, name) {
                        return Ok(false);
                    }

                    if !self.selection_set_is_valid_for_type(arena, name, node.children)? {
                        return Ok(false);
                    }
                }
                Kind::FragmentSpread => {
                    return Err(FetchGraphError::Internal(
                        "fragment spreads should have been inlined before abstract type conversion"
                            .to_string(),
                    ));
                }
            }
        }

        Ok(true)
    }

    /// Check that a fragment type is valid for its parent type.
    ///
    /// `... on Book` is valid inside `... on Media` only if `Book` implements `Media`.
    fn fragment_type_is_valid_for_parent_type(
        &self,
        parent_type_name: &str,
        fragment_type_name: &str,
    ) -> bool {
        if parent_type_name == fragment_type_name {
            return true;
        }

        self.subgraph
            .definitions
            .get(parent_type_name)
            .is_some_and(|definition| definition.is_interface_type())
            && self.object_is_interface_member_in_subgraph(fragment_type_name, parent_type_name)
    }

    /// Get information about an interface from the subgraph
    fn local_interface(&'a self, interface_name: &'a str) -> Option<InterfaceInSubgraph<'a>> {
        let definition = self.subgraph.definitions.get(interface_name)?;

        if !definition.is_interface_type() {
            return None;
        }

        let fields = definition.fields()?;
        let members = self.interface_object_members_in_subgraph(interface_name)?;

        if members.is_empty() {
            return None;
        }

        Some(InterfaceInSubgraph { fields, members })
    }

    /// Get the set of object types that implement an interface in the subgraph
    fn interface_object_members_in_subgraph(
        &'a self,
        interface_name: &str,
    ) -> Option<BTreeSet<&'a str>> {
        Some(
            self.supergraph
                .interface_members(interface_name)?
                .iter()
                .map(String::as_str)
                .filter(|name| self.object_is_interface_member_in_subgraph(name, interface_name))
                .collect(),
        )
    }

    /// Check whether an object type actually implements an interface in this subgraph
    fn object_is_interface_member_in_subgraph(
        &self,
        object_type_name: &str,
        interface_name: &str,
    ) -> bool {
        let Some(SupergraphDefinition::Object(object_type)) =
            self.supergraph.definitions.get(object_type_name)
        else {
            return false;
        };

        self.subgraph.definitions.contains_key(object_type_name)
            && object_type.join_implements.iter().any(|join_implements| {
                join_implements.graph_id == self.subgraph.graph_id
                    && join_implements.interface == interface_name
            })
    }

    fn object_type_exists_in_subgraph(&self, type_name: &str) -> bool {
        self.subgraph
            .definitions
            .get(type_name)
            .is_some_and(|definition| definition.is_object_type())
    }
}

impl MergePath {
    /// Walk through the response path segments and figure out what type we end up at.
    ///
    /// If the path is `["user", "friends"]` and the root is `"Query"`,
    /// we look up:
    ///   1. `Query.user` -> return type `User`
    ///   2. `User.friends` -> return type `[User]`, inner type `User`
    ///   3. Returns `User`.
    fn resolve_type_name<'a>(
        &'a self,
        root_type_name: &'a str,
        supergraph: &'a SupergraphState,
    ) -> Result<&'a str, FetchGraphError> {
        let mut type_name = root_type_name;

        for segment in self.inner.iter() {
            match segment {
                Segment::Field(field_seg, _, _) => {
                    let definition = supergraph.definitions.get(type_name).ok_or_else(|| {
                        FetchGraphError::Internal(format!(
                            "Type definition for {type_name} not found for given path {}",
                            self
                        ))
                    })?;
                    let field =
                        definition
                            .fields()
                            .get(field_seg.field_name())
                            .ok_or_else(|| {
                                FetchGraphError::Internal(format!(
                                    "Field {} not found in type {type_name} for given path {}",
                                    field_seg.field_name(),
                                    self
                                ))
                            })?;

                    type_name = field.field_type.inner_type();
                }
                Segment::List => {
                    // Lists don't change the type, we stay on the same type.
                }
                Segment::TypeCondition(type_names, _) => {
                    type_name = type_names.iter().next().ok_or_else(|| {
                        FetchGraphError::Internal(
                            "Expected at least one type in the type condition".to_string(),
                        )
                    })?;
                }
            }
        }

        Ok(type_name)
    }
}
