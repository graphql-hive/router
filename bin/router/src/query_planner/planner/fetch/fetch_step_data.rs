use crate::query_planner::planner::plan_nodes::FetchId;
use std::{collections::BTreeSet, fmt::Display};

use bitflags::bitflags;
use petgraph::graph::NodeIndex;

use crate::query_planner::{
    ast::{
        merge_path::{Condition, MergePath, Segment},
        operation::VariableDefinition,
        selection_set::SelectionSet,
    },
    planner::{
        fetch::{
            selections::{FetchStepSelections, FetchStepSelectionsError},
            state::{MultiTypeFetchStep, SingleTypeFetchStep},
        },
        plan_nodes::{FetchRewrite, KeyRenamer},
        tree::query_tree_node::MutationFieldPosition,
    },
    state::supergraph_state::{OperationKind, SubgraphName},
};

bitflags! {
    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct FetchStepFlags: u8 {
        /// This fetch is for resolving a @requires directive.
        const USED_FOR_REQUIRES = 1 << 0;
        /// This fetch is for resolving a type condition on an interface.
        const USED_FOR_TYPE_CONDITION = 1 << 1;
        /// Part of this call's input is only fetched under an `@include`/`@skip`. Set when calls
        /// with different conditions get merged, as the merged `condition` no longer shows it.
        const CONDITIONAL_INPUT = 1 << 2;
        /// Made to fetch what a `@requires` field needs, not the field itself. The steps
        /// `drop_covered_requirement_steps` looks at.
        const FETCHES_REQUIREMENTS = 1 << 3;
    }
}

#[derive(Debug, Clone)]
pub struct FetchStepData<State> {
    pub id: FetchId,
    pub service_name: SubgraphName,
    pub response_path: MergePath,
    pub input: FetchStepSelections<State>,
    pub output: FetchStepSelections<State>,
    pub kind: FetchStepKind,
    pub operation_kind: OperationKind,
    pub flags: FetchStepFlags,
    pub condition: Option<Condition>,
    /// The step only runs when all of these hold, top first. A step under
    /// `orders @include(if: $a) { ... on Order @include(if: $b) { name } }` only has `$b` as its
    /// `condition`, it relies on the `orders` step for `$a`, but it's gated by both. Merged with
    /// a step of another branch, only the conditions both have stay.
    pub gated_by: Vec<Condition>,
    pub variable_usages: Option<BTreeSet<String>>,
    pub variable_definitions: Option<Vec<VariableDefinition>>,
    pub mutation_field_position: MutationFieldPosition,
    pub input_rewrites: Option<Vec<FetchRewrite>>,
    pub output_rewrites: Option<Vec<FetchRewrite>>,
    pub context_rewrites: Option<Vec<KeyRenamer>>,
    pub context_variable_definitions: Vec<VariableDefinition>,
    pub context_producers: BTreeSet<FetchId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FetchStepKind {
    Entity,
    Root,
}

impl<State> Display for FetchStepData<State> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]: ", self.service_name)?;

        for (def_name, selections) in self.input.iter() {
            write!(f, "{}/{} ", def_name, selections)?;
        }

        write!(f, "-> ")?;

        for (def_name, selections) in self.output.iter() {
            write!(f, "{}/{} ", def_name, selections)?;
        }

        write!(f, "at $.{}", self.response_path.join("."))?;

        if self.flags.contains(FetchStepFlags::USED_FOR_REQUIRES) {
            write!(f, " [@requires]")?;
        }

        if self.flags.contains(FetchStepFlags::USED_FOR_TYPE_CONDITION) {
            write!(f, " [no_pass_through]")?;
        }

        if let Some(condition) = &self.condition {
            match condition {
                Condition::Include(var_name) => write!(f, " [@include(if: ${})]", var_name)?,
                Condition::Skip(var_name) => write!(f, " [@skip(if: ${})]", var_name)?,
                Condition::SkipAndInclude { skip, include } => {
                    write!(f, " [@skip(if: ${}) @include(if: ${})]", skip, include)?
                }
            }
        }

        Ok(())
    }
}

impl<State> FetchStepData<State> {
    pub fn pretty_write(
        &self,
        writer: &mut std::fmt::Formatter<'_>,
        index: NodeIndex,
    ) -> Result<(), std::fmt::Error> {
        write!(writer, "[{}] {}", index.index(), self)
    }

    pub fn is_entity_call(&self) -> bool {
        self.kind == FetchStepKind::Entity
    }

    pub fn add_input_rewrite(&mut self, rewrite: FetchRewrite) {
        let rewrites = self.input_rewrites.get_or_insert_default();

        if !rewrites.contains(&rewrite) {
            rewrites.push(rewrite);
        }
    }

    pub fn add_output_rewrite(&mut self, rewrite: FetchRewrite) {
        let rewrites = self.output_rewrites.get_or_insert_default();

        if !rewrites.contains(&rewrite) {
            rewrites.push(rewrite);
        }
    }
}

impl<State> FetchStepData<State> {
    /// Where the type condition that picks one of our types sits in a path at or under ours:
    /// our last segment when we end with one, like `search.@|[Book|Movie]`, or the segment right
    /// after our path once that's every type, like `search.@`. With where the rest starts.
    fn type_slot(&self) -> (usize, usize) {
        let ours = self.response_path.len();
        match self.response_path.last() {
            Some(Segment::TypeCondition(..)) => (ours - 1, ours),
            _ => (ours, ours + 1),
        }
    }

    pub fn is_fetching_multiple_types(&self) -> bool {
        self.input.is_fetching_multiple_types() || self.output.is_fetching_multiple_types()
    }
}

// Extracts concrete type names from the type condition the response path ends with, the types
// the step's output is about.
// `products.@|[Book|Magazine]` gives back `Book` and `Magazine`, `similar|[Book].reviews` nothing,
// `[Book]` is about `similar` there, not about the reviews.
pub(crate) fn type_condition_types_from_response_path(
    response_path: &MergePath,
) -> Option<BTreeSet<&str>> {
    match response_path.last() {
        Some(Segment::TypeCondition(type_names, _)) => Some(
            type_names
                .iter()
                .map(|type_name| type_name.as_str())
                .collect(),
        ),
        _ => None,
    }
}

/// Where the output of a step deeper in the response goes in ours, see `place_of_nested`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NestedPlace<'a> {
    /// In `type_name`'s selections, at the nested path from `rest` on.
    Type { type_name: &'a str, rest: usize },
    /// At our own spot, each type in its own selections.
    EachType,
}

impl FetchStepData<MultiTypeFetchStep> {
    /// Where the output of a step deeper in the response, at `nested_path`, goes in ours. With
    /// one type, in ours. With several, like `search.@|[Book|Movie]` or `search.@` once that's
    /// every type, the nested path says which: `search.@|[Movie].director` goes in `Movie`, at
    /// `director`. At our own spot each type goes in its own. `None` when the path doesn't say,
    /// or isn't under ours.
    pub(crate) fn place_of_nested<'a>(
        &'a self,
        nested_path: &'a MergePath,
    ) -> Option<NestedPlace<'a>> {
        let ours = self.response_path.len();
        if nested_path.len() < ours {
            return None;
        }
        if let Some(type_name) = self.output.try_as_single() {
            return Some(NestedPlace::Type {
                type_name,
                rest: ours,
            });
        }
        if nested_path.len() == ours {
            return Some(NestedPlace::EachType);
        }
        let (slot, rest) = self.type_slot();
        match nested_path.inner.get(slot) {
            Some(Segment::TypeCondition(type_names, _)) if type_names.len() == 1 => {
                let type_name = type_names.first()?;
                self.output
                    .is_selecting_definition(type_name)
                    .then_some(NestedPlace::Type { type_name, rest })
            }
            _ => None,
        }
    }

    // Moves a fetch-level condition down into this step's output selections.
    // A fetch-level condition means "the whole HTTP request can be skipped".
    // That is only correct while every output selection in the fetch has the same condition.
    // Before merging a conditional fetch with an unconditional or differently-conditional fetch,
    // the condition must be scoped to the affected output selections so it cannot leak to merged siblings.
    fn scope_condition_to_output(&mut self, scope_by_response_path_types: bool) {
        let Some(condition) = self.condition.take() else {
            return;
        };

        let type_names = if scope_by_response_path_types {
            type_condition_types_from_response_path(&self.response_path)
        } else {
            None
        };

        // If concrete type names are known, scope the condition to those types only.
        // Otherwise apply it to every output type in this fetch.
        if let Some(types) = type_names {
            self.output.wrap_with_condition_for_types(condition, &types);
        } else {
            self.output.wrap_with_condition(condition);
        }
    }

    pub(crate) fn scope_fetch_conditions_before_merge(&mut self, source: &mut Self) {
        if !self.is_entity_call() {
            return;
        }

        let can_scope_by_type =
            self.is_fetching_multiple_types() || source.is_fetching_multiple_types();
        let condition_is_type_scoped = self.condition.is_some()
            && self.is_fetching_multiple_types()
            && type_condition_types_from_response_path(&self.response_path).is_some();

        // A condition carried by a typed response path must stay on that concrete branch.
        // For example, `products.@|[Book]` with `@include($showBook)` must not become an `Include` node
        // around a merged Book+Magazine fetch, or Magazine data would disappear when false.
        if condition_is_type_scoped {
            self.scope_condition_to_output(true);
        }

        // If both sides still have the same condition, keep it at fetch level,
        // so the whole merged HTTP request can be skipped.
        if self.condition != source.condition {
            self.scope_condition_to_output(can_scope_by_type);
            source.scope_condition_to_output(can_scope_by_type);
        }
    }

    pub(crate) fn lift_shared_output_condition_to_fetch(&mut self) {
        // After a safe merge, all output selections may again be guarded by the
        // same shared condition.
        // Both `name` and `isCheap` may have `@include($showDetails)`.
        // In that case we lift the condition back to fetch level so the router can skip the whole HTTP request.
        if self.condition.is_none() {
            self.condition = self.output.take_shared_top_level_fragment_condition();
        }
    }
}

impl FetchStepData<SingleTypeFetchStep> {
    /// Whether an entity call to this subgraph, type, path and condition could reuse this step.
    pub fn matches_entity_call(
        &self,
        subgraph_name: &SubgraphName,
        input_type_name: &str,
        response_path: &MergePath,
        condition: Option<&Condition>,
    ) -> bool {
        self.service_name == *subgraph_name
            && self.input.first_definition_name() == Some(input_type_name)
            && self.response_path == *response_path
            && self.condition.as_ref() == condition
    }

    /// The types the objects at `response_path` have in our output, so which selection sets what
    /// goes there lands in. A step fetches one type, except an entity call that takes the
    /// objects of several types at one spot, like `search.@|[Book|Movie]`. There it's the types
    /// `response_path` has at the end of our path: one, or several planned together, see
    /// `SelectionAttributes::also`. Empty for the only type.
    fn output_types_at<'a>(
        &self,
        response_path: &'a MergePath,
    ) -> Result<Option<&'a BTreeSet<String>>, FetchStepSelectionsError> {
        if !self.output.is_fetching_multiple_types() {
            return Ok(None);
        }
        let (slot, _) = self.type_slot();
        match response_path.inner.get(slot) {
            Some(Segment::TypeCondition(type_names, _)) => Ok(Some(type_names)),
            _ => Err(FetchStepSelectionsError::UnexpectedMissingDefinition(
                format!(
                    "no type at {response_path} in a step at {}",
                    self.response_path
                ),
            )),
        }
    }

    /// Adds `selection_set` at `fetch_path` of our output, for the objects at `response_path`.
    pub fn add_output_at(
        &mut self,
        response_path: &MergePath,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
    ) -> Result<(), FetchStepSelectionsError> {
        match self.output_types_at(response_path)? {
            None => self.output.add_at_path(None, fetch_path, selection_set),
            Some(type_names) => {
                for type_name in type_names {
                    self.output
                        .add_at_path(Some(type_name), fetch_path, selection_set.clone())?;
                }
                Ok(())
            }
        }
    }

    /// Adds `__typename` at `fetch_path` of our output, for the objects at `response_path`.
    pub fn add_output_typename_at(
        &mut self,
        response_path: &MergePath,
        fetch_path: &MergePath,
    ) -> Result<(), FetchStepSelectionsError> {
        match self.output_types_at(response_path)? {
            None => self.output.add_selection_typename(None, fetch_path),
            Some(type_names) => {
                for type_name in type_names {
                    self.output
                        .add_selection_typename(Some(type_name), fetch_path)?;
                }
                Ok(())
            }
        }
    }

    pub fn into_multi_type(self) -> FetchStepData<MultiTypeFetchStep> {
        FetchStepData::<MultiTypeFetchStep> {
            id: self.id,
            service_name: self.service_name,
            response_path: self.response_path,
            input: self.input.into_multi_type(),
            output: self.output.into_multi_type(),
            kind: self.kind,
            operation_kind: self.operation_kind,
            flags: self.flags,
            condition: self.condition,
            gated_by: self.gated_by,
            variable_usages: self.variable_usages,
            variable_definitions: self.variable_definitions,
            mutation_field_position: self.mutation_field_position,
            input_rewrites: self.input_rewrites,
            output_rewrites: self.output_rewrites,
            context_rewrites: self.context_rewrites,
            context_variable_definitions: self.context_variable_definitions,
            context_producers: self.context_producers,
        }
    }
}
