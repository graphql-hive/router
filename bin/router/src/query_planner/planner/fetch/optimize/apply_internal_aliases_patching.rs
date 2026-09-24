use std::collections::BTreeSet;

use petgraph::{
    graph::NodeIndex,
    visit::{Bfs, IntoNodeReferences},
};
use tracing::{instrument, trace};

use crate::query_planner::{
    ast::{
        merge_path::{Condition, MergePath, Segment},
        selection_item::SelectionItem,
        selection_set::find_selection_set_by_path_mut,
    },
    planner::fetch::{
        error::FetchGraphError,
        fetch_graph::FetchGraph,
        fetch_step_data::{FetchStepData, InternalAlias},
        state::MultiTypeFetchStep,
    },
    state::supergraph_state::SupergraphState,
};

impl FetchGraph<MultiTypeFetchStep> {
    /// A step that fetches a field under an internal alias has to tell the steps that read
    /// that field. They all run after it, so they're among its descendants, and they can read
    /// it in two ways:
    /// 1. In their input, when it's part of what they send to their subgraph.
    /// 2. In their `response_path`, when the objects they resolve sit under that field.
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn apply_internal_aliases_patching(
        &mut self,
        supergraph: &SupergraphState,
    ) -> Result<(), FetchGraphError> {
        let steps_with_aliases = self
            .graph
            .node_references()
            .filter(|(_, step)| !step.internal_aliases.is_empty())
            .map(|(index, step)| (index, step.internal_aliases.clone(), step.condition.clone()))
            .collect::<Vec<_>>();

        trace!(
            "found total of {} steps with internal aliased fields",
            steps_with_aliases.len(),
        );

        for (aliased_step_index, aliases, aliased_step_condition) in steps_with_aliases {
            let mut bfs = Bfs::new(&self.graph, aliased_step_index);
            while let Some(descendant_index) = bfs.next(&self.graph) {
                if descendant_index == aliased_step_index {
                    continue;
                }
                let descendant = self.get_step_data_mut(descendant_index)?;
                for alias in &aliases {
                    patch_reader(
                        descendant,
                        descendant_index,
                        alias,
                        aliased_step_condition.as_ref(),
                        supergraph,
                    )?;
                }
            }
        }

        Ok(())
    }
}

fn patch_reader(
    reader: &mut FetchStepData<MultiTypeFetchStep>,
    reader_index: NodeIndex,
    alias: &InternalAlias,
    aliased_step_condition: Option<&Condition>,
    supergraph: &SupergraphState,
) -> Result<(), FetchGraphError> {
    let types_overlap = |a: &[&BTreeSet<String>], b: &[&BTreeSet<String>]| {
        !objects_of(supergraph, a).is_disjoint(&objects_of(supergraph, b))
    };

    // The alias is only in the response when every condition on its way is true. A reader
    // that's sent in other cases too can't count on it, it reads the plain field.
    let needed = conditions_of(&alias.location, aliased_step_condition);
    let given = conditions_of(&reader.response_path, reader.condition.as_ref());
    if !needed.is_subset(&given) {
        trace!(
            "step [{}] is sent under other conditions than alias '{}' at '{}'",
            reader_index.index(),
            alias.alias,
            alias.location
        );
        return Ok(());
    }

    let Some(Segment::Field(field_seg, args_hash, condition)) = alias.location.last() else {
        return Err(FetchGraphError::Internal(format!(
            "Internal alias '{}' doesn't end with a field: {}",
            alias.alias, alias.location
        )));
    };
    // Where the object holding the aliased field sits in the response.
    let parent_location = alias.location.without_last();

    // The input can only read the field when the reader resolves objects at or above that
    // parent. A step batched for many types has a selection per type, and each one only reads
    // objects of its type.
    let input_types: Vec<String> = reader
        .input
        .iter_selections()
        .map(|(t, _)| t.clone())
        .collect();
    let batched = input_types.len() > 1;
    for type_name in input_types {
        let reader_location = if batched {
            reader.response_path.push(Segment::TypeCondition(
                BTreeSet::from([type_name.clone()]),
                None,
            ))
        } else {
            reader.response_path.clone()
        };
        let Some(rest) = parent_location.strip_location_prefix(&reader_location, types_overlap)
        else {
            continue;
        };
        // Inputs have no fragments for the types the path narrows down to.
        let relative_path = parent_location.slice_from(rest).without_type_castings();
        let selection = reader
            .input
            .selections_for_definition_mut(&type_name)
            .ok_or_else(|| {
                FetchGraphError::Internal(format!(
                    "Missing input selections of type {} in step [{}]",
                    type_name,
                    reader_index.index()
                ))
            })?;

        if let Some(selection) = find_selection_set_by_path_mut(selection, &relative_path) {
            let field_to_patch = selection.items.iter_mut().find_map(|item| match item {
                SelectionItem::Field(field)
                    if field.name == field_seg.field_name()
                        && field.arguments_hash() == *args_hash =>
                {
                    Some(field)
                }
                _ => None,
            });
            if let Some(field) = field_to_patch {
                field.alias = Some(field.name.clone());
                field.name = alias.alias.clone();
                trace!(
                    "patched input of step [{}] at '{}': {}",
                    reader_index.index(),
                    parent_location,
                    field
                );
            }
        }
    }

    // Only the segment at the aliased field's own position counts, an ancestor with the same
    // name and arguments is a different field.
    let segment_idx_to_patch = reader
        .response_path
        .strip_location_prefix(&alias.location, types_overlap)
        // The segment right before the rest is the aliased field.
        .map(|rest| rest - 1)
        .filter(|idx| {
            matches!(&reader.response_path.inner[*idx], Segment::Field(_, _, c) if c == condition)
        });

    if let Some(idx) = segment_idx_to_patch {
        let mut new_path = reader.response_path.inner.to_vec();
        if let Some(Segment::Field(segment, _, _)) = new_path.get_mut(idx) {
            segment.field_name = alias.alias.clone();
            reader.response_path = MergePath::new(new_path);
            trace!(
                "patched response path of step [{}]: {}",
                reader_index.index(),
                reader.response_path
            );
        }
    }

    Ok(())
}

/// The `@include`/`@skip` on a path and on its step, as (variable, is a skip) pairs.
fn conditions_of<'a>(
    path: &'a MergePath,
    step_condition: Option<&'a Condition>,
) -> BTreeSet<(&'a str, bool)> {
    let on_path = path.inner.iter().filter_map(|segment| match segment {
        Segment::Field(_, _, condition) | Segment::TypeCondition(_, condition) => {
            condition.as_ref()
        }
        Segment::List => None,
    });
    on_path
        .chain(step_condition)
        .flat_map(|condition| match condition {
            Condition::Include(variable) => vec![(variable.as_str(), false)],
            Condition::Skip(variable) => vec![(variable.as_str(), true)],
            Condition::SkipAndInclude { skip, include } => {
                vec![(skip.as_str(), true), (include.as_str(), false)]
            }
        })
        .collect()
}

/// The object types a run of type conditions lets through. Each one lets through what its
/// types can be, and the ones in a row narrow it down: `|[Node]|[Cat]` is only `Cat`s.
fn objects_of<'a>(
    supergraph: &'a SupergraphState,
    type_conditions: &[&'a BTreeSet<String>],
) -> BTreeSet<&'a str> {
    type_conditions
        .iter()
        .map(|names| {
            names
                .iter()
                .flat_map(|name| supergraph.possible_object_types(name))
                .collect::<BTreeSet<_>>()
        })
        .reduce(|a, b| a.intersection(&b).copied().collect())
        .unwrap_or_default()
}
