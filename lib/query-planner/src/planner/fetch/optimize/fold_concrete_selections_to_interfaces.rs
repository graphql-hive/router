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
use std::collections::BTreeSet;

use tracing::instrument;

use crate::{
    ast::{
        merge_path::{Condition, MergePath, Segment},
        selection_item::SelectionItem,
        selection_set::{merge_selection_set, InlineFragmentSelection, SelectionSet},
        semantic_eq::SemanticEq,
    },
    planner::fetch::{
        error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
        state::MultiTypeFetchStep,
    },
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
            let step = self.get_step_data_mut(index)?;
            let Ok(subgraph) = supergraph.subgraph_state(&step.service_name) else {
                continue;
            };

            changed |= StepConverter {
                root_type_name,
                supergraph,
                subgraph: subgraph,
            }
            .rewrite_step(step)?;
        }

        Ok(changed)
    }
}

struct StepConverter<'a> {
    root_type_name: &'a str,
    supergraph: &'a SupergraphState,
    subgraph: &'a SubgraphState,
}

struct InterfaceInSubgraph<'a> {
    fields: &'a [SubgraphField],
    members: BTreeSet<&'a str>,
}

/// Object-type branch that we might try to fold into an interface
struct ObjectTypeBranch<'a> {
    type_name: &'a str,
    selection_set: &'a SelectionSet,
    condition: Option<Condition>,
    remove_index: Option<usize>,
}

/// The result of folding concrete object type into one interface
struct FoldedSelection {
    selection_set: SelectionSet,
    condition: Option<Condition>,
    remove_indexes: Vec<usize>,
}

impl<'a> StepConverter<'a> {
    fn rewrite_step(
        &self,
        step: &mut FetchStepData<MultiTypeFetchStep>,
    ) -> Result<bool, FetchGraphError> {
        let mut changed = false;
        changed |= self.rewrite_root_output(step)?;
        changed |= self.rewrite_nested_output(step)?;
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
        step: &mut FetchStepData<MultiTypeFetchStep>,
    ) -> Result<bool, FetchGraphError> {
        // Only rewrite if the step is at the root (no response path).
        if !step.response_path.inner.is_empty() {
            return Ok(false);
        }

        let interface_name = step
            .response_path
            .resolve_type_name(self.root_type_name, self.supergraph)?;

        let branches = step
            .output
            .iter_selections()
            .map(|(type_name, selection_set)| ObjectTypeBranch {
                type_name: type_name.as_str(),
                selection_set,
                condition: None,
                remove_index: None,
            })
            .collect::<Vec<_>>();

        let Some(folded) = self.try_fold_into_interface(interface_name, branches)? else {
            return Ok(false);
        };

        step.output
            .replace_definitions_with_abstract(interface_name, folded.selection_set);

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
        step: &mut FetchStepData<MultiTypeFetchStep>,
    ) -> Result<bool, FetchGraphError> {
        let mut changed = false;

        for (definition_name, selection_set) in step.output.iter_selections_mut() {
            changed |= self.rewrite_selection_set(definition_name, selection_set)?;
        }

        Ok(changed)
    }

    fn rewrite_selection_set(
        &self,
        current_type_name: &str,
        selection_set: &mut SelectionSet,
    ) -> Result<bool, FetchGraphError> {
        let mut changed = false;

        for item in &mut selection_set.items {
            match item {
                SelectionItem::Field(field) => {
                    let child_type_name = self
                        .supergraph
                        .field_return_type_name(current_type_name, field.name.as_str())
                        .ok_or_else(|| {
                            FetchGraphError::Internal(format!(
                                "No field found for name '{}' in type '{}'",
                                field.name, current_type_name
                            ))
                        })?;
                    changed |=
                        self.rewrite_selection_set(child_type_name, &mut field.selections)?;
                }
                SelectionItem::InlineFragment(fragment) => {
                    changed |= self.rewrite_selection_set(
                        &fragment.type_condition,
                        &mut fragment.selections,
                    )?;
                }
                SelectionItem::FragmentSpread(_) => {
                    // Fragment spreads should have been inlined before we get here.
                    // If we see one, something went wrong earlier.
                    return Err(FetchGraphError::Internal(
                        "fragment spreads should have been inlined before abstract type conversion"
                            .to_string(),
                    ));
                }
            }
        }
        changed |= self.rewrite_inline_fragments(current_type_name, selection_set)?;

        Ok(changed)
    }

    fn rewrite_inline_fragments(
        &self,
        current_type_name: &str,
        selection_set: &mut SelectionSet,
    ) -> Result<bool, FetchGraphError> {
        let candidate = {
            let branches = selection_set
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| match item {
                    SelectionItem::InlineFragment(fragment) => Some(ObjectTypeBranch {
                        type_name: fragment.type_condition.as_str(),
                        selection_set: &fragment.selections,
                        condition: Option::<Condition>::from(fragment),
                        remove_index: Some(index),
                    }),
                    _ => None,
                })
                .collect::<Vec<_>>();

            self.try_fold_into_interface(current_type_name, branches)?
        };

        let Some(candidate) = candidate else {
            return Ok(false);
        };

        // Remove the branches we folded
        selection_set.items = selection_set
            .items
            .drain(..)
            .enumerate()
            .filter_map(|(index, item)| {
                if candidate.remove_indexes.contains(&index) {
                    None // drop
                } else {
                    Some(item) // keep
                }
            })
            .collect();

        if let Some(condition) = candidate.condition {
            // If the original branches had @skip/@include
            selection_set
                .items
                .push(SelectionItem::InlineFragment(InlineFragmentSelection {
                    type_condition: current_type_name.to_string(),
                    selections: candidate.selection_set,
                    skip_if: condition.to_skip_if(),
                    include_if: condition.to_include_if(),
                }));
        } else {
            // Otherwise merge the fields directly into the parent selection set
            merge_selection_set(selection_set, &candidate.selection_set, false);
        }

        Ok(true)
    }

    fn try_fold_into_interface(
        &self,
        interface_name: &str,
        branches: Vec<ObjectTypeBranch<'_>>,
    ) -> Result<Option<FoldedSelection>, FetchGraphError> {
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

        for branch in &branches {
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

        let selection_set = first_branch.selection_set;

        if !branches
            .iter()
            .skip(1)
            // All must branches ask for the same fields
            .all(|branch| selection_set.semantic_eq(branch.selection_set))
        {
            return Ok(None);
        }

        if !self.selected_fields_exist_on_interface(
            first_branch.type_name,
            selection_set,
            interface.fields,
        ) {
            // The selected fields do not exist on the interface
            return Ok(None);
        }

        if !self.selection_set_is_valid_for_type(interface_name, selection_set)? {
            // The selection set (fields and fragments) is not valid for the interface
            return Ok(None);
        }

        Ok(Some(FoldedSelection {
            selection_set: selection_set.clone(),
            condition: shared_condition,
            remove_indexes: branches
                .iter()
                .filter_map(|branch| branch.remove_index)
                .collect(),
        }))
    }

    /// Check that every field exists on the interface.
    /// If a field exists on the object type but not on the interface, we cannot fold.
    fn selected_fields_exist_on_interface(
        &self,
        type_name: &str,
        selection_set: &SelectionSet,
        interface_fields: &[SubgraphField],
    ) -> bool {
        selection_set
            .iter_fields_and_fragments_of_same_type(type_name)
            .all(|field| {
                field.name == "__typename"
                    || interface_fields
                        .iter()
                        .any(|interface_field| interface_field.name == field.name)
            })
    }

    /// Check that a selection set is valid for a given type in this subgraph.
    fn selection_set_is_valid_for_type(
        &self,
        type_name: &str,
        selection_set: &SelectionSet,
    ) -> Result<bool, FetchGraphError> {
        if selection_set.items.is_empty() {
            return Ok(true);
        }

        let Some(definition) = self.subgraph.definitions.get(type_name) else {
            return Ok(false);
        };

        for item in &selection_set.items {
            match item {
                SelectionItem::Field(field) => {
                    if field.name == "__typename" {
                        continue;
                    }

                    let Some(field_definition) = definition.field(field.name.as_str()) else {
                        return Ok(false);
                    };

                    if !self.selection_set_is_valid_for_type(
                        field_definition.field_type.inner_type(),
                        &field.selections,
                    )? {
                        return Ok(false);
                    }
                }
                SelectionItem::InlineFragment(fragment) => {
                    if !self
                        .fragment_type_is_valid_for_parent_type(type_name, &fragment.type_condition)
                    {
                        return Ok(false);
                    }

                    if !self.selection_set_is_valid_for_type(
                        &fragment.type_condition,
                        &fragment.selections,
                    )? {
                        return Ok(false);
                    }
                }
                SelectionItem::FragmentSpread(_) => {
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
    /// Returns `User`.
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
                            "Type definition for {type_name} not found"
                        ))
                    })?;
                    let field =
                        definition
                            .fields()
                            .get(field_seg.field_name())
                            .ok_or_else(|| {
                                FetchGraphError::Internal(format!(
                                    "Field {} not found in type {type_name}",
                                    field_seg.field_name()
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
