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
        selection_set::{segment_selects, SelectionSet},
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
                for alias in &aliases {
                    let descendant = self.get_step_data_mut(descendant_index)?;
                    let patched_path = patch_reader(
                        descendant,
                        descendant_index,
                        alias,
                        aliased_step_condition.as_ref(),
                        supergraph,
                    )?;
                    if let Some(path) = patched_path {
                        let location = self.locations.get(&path);
                        self.get_step_data_mut(descendant_index)?.response_path = location;
                    }
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
) -> Result<Option<MergePath>, FetchGraphError> {
    let types_overlap = |a: &[&BTreeSet<String>], b: &[&BTreeSet<String>]| {
        !objects_of(supergraph, a).is_disjoint(&objects_of(supergraph, b))
    };

    // The alias is only in the response when every condition on its way is true. A reader
    // that's sent in other cases too can't count on it, it reads the plain field.
    let needed = conditions_of(&alias.location, aliased_step_condition);
    let given = conditions_of(reader.response_path.path(), reader.condition.as_ref());
    if !needed.is_subset(&given) {
        trace!(
            "step [{}] is sent under other conditions than alias '{}' at '{}'",
            reader_index.index(),
            alias.alias,
            alias.location
        );
        return Ok(None);
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
            reader.response_path.path().push(Segment::TypeCondition(
                BTreeSet::from([type_name.clone()]),
                None,
            ))
        } else {
            reader.response_path.path().clone()
        };
        let Some(rest) = parent_location.strip_location_prefix(&reader_location, types_overlap)
        else {
            continue;
        };
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
        patch_input(
            selection,
            &parent_location.inner[rest..],
            None,
            (field_seg.field_name(), *args_hash),
            &alias.alias,
            supergraph,
        );
        trace!(
            "patched input of step [{}] at '{}': {}",
            reader_index.index(),
            parent_location,
            selection
        );
    }

    // Only the segment at the aliased field's own position counts, an ancestor with the same
    // name and arguments is a different field.
    let response_path = reader.response_path.path();
    let segment_idx_to_patch = response_path
        .strip_location_prefix(&alias.location, types_overlap)
        // The segment right before the rest is the aliased field.
        .map(|rest| rest - 1)
        .filter(
            |idx| matches!(&response_path.inner[*idx], Segment::Field(_, _, c) if c == condition),
        );

    let Some(idx) = segment_idx_to_patch else {
        return Ok(None);
    };
    let mut new_path = response_path.inner.to_vec();
    let Some(Segment::Field(segment, _, _)) = new_path.get_mut(idx) else {
        return Ok(None);
    };
    segment.field_name = alias.alias.clone();
    let new_path = MergePath::new(new_path);
    trace!(
        "patched response path of step [{}]: {}",
        reader_index.index(),
        new_path
    );
    Ok(Some(new_path))
}

/// Renames the field to the alias wherever the input reads it at `path`. The input can wrap it
/// in fragments, like `... on Node { ... on Cat { price } }`, so we look inside those, but only
/// the ones for types that can have the alias. `objects` is what the path lets through at this
/// level, `None` when it says nothing.
fn patch_input(
    selection_set: &mut SelectionSet,
    path: &[Segment],
    objects: Option<&BTreeSet<&str>>,
    (field_name, args_hash): (&str, u64),
    alias: &str,
    supergraph: &SupergraphState,
) {
    // Type conditions before the next field narrow down the objects at this level.
    let level_len = path
        .iter()
        .take_while(|segment| !matches!(segment, Segment::Field(..)))
        .count();
    let type_conditions: Vec<_> = path[..level_len]
        .iter()
        .filter_map(|segment| match segment {
            Segment::TypeCondition(types, _) => Some(types),
            _ => None,
        })
        .collect();
    let narrowed;
    let objects = if type_conditions.is_empty() {
        objects
    } else {
        narrowed = objects_of(supergraph, &type_conditions);
        Some(&narrowed)
    };
    let path = &path[level_len..];

    for item in selection_set.items.iter_mut() {
        let descend = path
            .first()
            .is_some_and(|segment| segment_selects(segment, item));
        match item {
            SelectionItem::InlineFragment(fragment) => {
                let fits = objects.is_none_or(|objects| {
                    supergraph
                        .possible_object_types(&fragment.type_condition)
                        .iter()
                        .any(|object| objects.contains(object))
                });
                if fits {
                    patch_input(
                        &mut fragment.selections,
                        path,
                        objects,
                        (field_name, args_hash),
                        alias,
                        supergraph,
                    );
                }
            }
            SelectionItem::Field(field) if path.is_empty() => {
                if field.name == field_name && field.arguments_hash() == args_hash {
                    field.alias = Some(field.name.clone());
                    field.name = alias.to_string();
                }
            }
            SelectionItem::Field(field) if descend => {
                patch_input(
                    &mut field.selections,
                    &path[1..],
                    None,
                    (field_name, args_hash),
                    alias,
                    supergraph,
                );
            }
            _ => {}
        }
    }
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
