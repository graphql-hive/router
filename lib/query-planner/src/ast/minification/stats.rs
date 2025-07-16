use std::collections::HashMap;

use crate::ast::minification::error::MinificationError;
use crate::ast::minification::selection_id::{generate_selection_id, SelectionId};
use crate::ast::selection_item::SelectionItem;
use crate::ast::selection_set::SelectionSet;
use crate::state::supergraph_state::SupergraphState;

pub struct Stats {
    state: HashMap<SelectionId, usize>,
    contains_duplicates: bool,
}

impl Stats {
    pub fn from_operation(
        selection_set: &SelectionSet,
        supergraph: &SupergraphState,
        root_type_name: &str,
    ) -> Result<Self, MinificationError> {
        let mut stats = Self {
            state: HashMap::new(),
            contains_duplicates: false,
        };

        walk_and_collect_stats(&mut stats, supergraph, selection_set, root_type_name)?;
        Ok(stats)
    }

    pub fn has_duplicates(&self) -> bool {
        self.contains_duplicates
    }

    pub fn is_duplicated(&self, key: &SelectionId) -> bool {
        self.state.get(key).unwrap_or(&0) > &1
    }

    pub fn increase(&mut self, key: SelectionId) -> &usize {
        let occurrences = self.state.entry(key).or_insert_with(|| 0);
        *occurrences += 1;

        if *occurrences > 1 {
            self.contains_duplicates = true;
        }

        occurrences
    }
}

fn walk_and_collect_stats(
    stats: &mut Stats,
    supergraph: &SupergraphState,
    selection_set: &SelectionSet,
    type_name: &str,
) -> Result<(), MinificationError> {
    if selection_set.items.is_empty() {
        return Ok(());
    }

    let id = generate_selection_id(type_name, selection_set);
    let occurrences = stats.increase(id);

    if *occurrences > 1 {
        return Ok(());
    }

    let type_def = supergraph
        .definitions
        .get(type_name)
        .ok_or_else(|| MinificationError::TypeNotFound(type_name.to_string()))?;

    for item in &selection_set.items {
        match item {
            SelectionItem::Field(field) => {
                if field.is_introspection_field() {
                    continue;
                }

                // Special case where _entities is used,
                // and as you know, Query._entities is not part of the schema,
                // so we need to handle it specially.
                // The content of _entities is a list of inline fragments.
                // We can extract the type condition and pass it as the type of the selection set.
                // This way we don't lookup the type in the schema.
                if field.name == "_entities" {
                    for item in &field.selections.items {
                        match item {
                            SelectionItem::Field(field) => {
                                if field.is_introspection_field() {
                                    continue;
                                }

                                return Err(MinificationError::UnsupportedFieldInEntities(
                                    field.name.clone(),
                                ));
                            }
                            SelectionItem::InlineFragment(fragment) => {
                                walk_and_collect_stats(
                                    stats,
                                    supergraph,
                                    &fragment.selections,
                                    &fragment.type_condition,
                                )?;
                            }
                            SelectionItem::FragmentSpread(_) => {
                                return Err(MinificationError::UnsupportedFragmentSpread);
                            }
                        }
                    }
                    continue;
                }

                let child_type_name = type_def
                    .fields()
                    .get(&field.name)
                    .ok_or_else(|| {
                        MinificationError::FieldNotFound(
                            field.name.clone(),
                            type_def.name().to_string(),
                        )
                    })?
                    .field_type
                    .inner_type();
                walk_and_collect_stats(stats, supergraph, &field.selections, child_type_name)?;
            }
            SelectionItem::InlineFragment(fragment) => {
                walk_and_collect_stats(
                    stats,
                    supergraph,
                    &fragment.selections,
                    &fragment.type_condition,
                )?;
            }
            SelectionItem::FragmentSpread(_) => {
                return Err(MinificationError::UnsupportedFragmentSpread);
            }
        }
    }

    Ok(())
}
