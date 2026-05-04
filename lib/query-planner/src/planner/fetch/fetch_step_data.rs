use std::{collections::BTreeSet, fmt::Display};

use bitflags::bitflags;
use petgraph::graph::NodeIndex;

use crate::{
    ast::{
        merge_path::{Condition, MergePath, Segment},
        operation::VariableDefinition,
        safe_merge::AliasesRecords,
    },
    planner::{
        fetch::{
            selections::FetchStepSelections,
            state::{MultiTypeFetchStep, SingleTypeFetchStep},
        },
        plan_nodes::FetchRewrite,
        tree::query_tree_node::MutationFieldPosition,
    },
    state::supergraph_state::SubgraphName,
};

bitflags! {
    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct FetchStepFlags: u8 {
        /// This fetch is for resolving a @requires directive.
        const USED_FOR_REQUIRES = 1 << 0;
        /// This fetch is for resolving a type condition on an interface.
        const USED_FOR_TYPE_CONDITION = 1 << 1;
    }
}

#[derive(Debug, Clone)]
pub struct FetchStepData<State> {
    pub id: i64,
    pub service_name: SubgraphName,
    pub response_path: MergePath,
    pub input: FetchStepSelections<State>,
    pub output: FetchStepSelections<State>,
    pub kind: FetchStepKind,
    pub flags: FetchStepFlags,
    pub condition: Option<Condition>,
    pub variable_usages: Option<BTreeSet<String>>,
    pub variable_definitions: Option<Vec<VariableDefinition>>,
    pub mutation_field_position: MutationFieldPosition,
    pub input_rewrites: Option<Vec<FetchRewrite>>,
    pub output_rewrites: Option<Vec<FetchRewrite>>,
    pub internal_aliases_locations: Vec<(String, AliasesRecords)>,
}

#[derive(Debug, Clone, PartialEq)]
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
    pub fn is_fetching_multiple_types(&self) -> bool {
        self.input.is_fetching_multiple_types() || self.output.is_fetching_multiple_types()
    }
}

// Extracts concrete type names from response-path type-condition segments.
// `products.@|[Book|Magazine]` gives back `Book` and `Magazine`
pub(crate) fn type_condition_types_from_response_path(
    response_path: &MergePath,
) -> Option<BTreeSet<&str>> {
    let conditioned_types = response_path
        .inner
        .iter()
        .filter_map(|segment| match segment {
            Segment::TypeCondition(type_names, _) => Some(type_names),
            _ => None,
        })
        .flat_map(|type_names| type_names.iter().map(|type_name| type_name.as_str()))
        .collect::<BTreeSet<_>>();

    if conditioned_types.is_empty() {
        None
    } else {
        Some(conditioned_types)
    }
}

impl FetchStepData<MultiTypeFetchStep> {
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
    pub fn into_multi_type(self) -> FetchStepData<MultiTypeFetchStep> {
        FetchStepData::<MultiTypeFetchStep> {
            id: self.id,
            service_name: self.service_name,
            response_path: self.response_path,
            input: self.input.into_multi_type(),
            output: self.output.into_multi_type(),
            kind: self.kind,
            flags: self.flags,
            condition: self.condition,
            variable_usages: self.variable_usages,
            variable_definitions: self.variable_definitions,
            mutation_field_position: self.mutation_field_position,
            input_rewrites: self.input_rewrites,
            output_rewrites: self.output_rewrites,
            internal_aliases_locations: self.internal_aliases_locations,
        }
    }
}
