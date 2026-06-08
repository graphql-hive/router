use tracing::instrument;

use crate::{
    ast::{
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
    },
    planner::fetch::{
        error::FetchGraphError, fetch_graph::FetchGraph, fetch_step_data::FetchStepData,
        state::MultiTypeFetchStep,
    },
    state::supergraph_state::SupergraphState,
};

impl FetchGraph<MultiTypeFetchStep> {
    #[instrument(level = "trace", skip_all)]
    pub(crate) fn normalize_selection_sets(
        &mut self,
        supergraph: &SupergraphState,
    ) -> Result<(), FetchGraphError> {
        let step_indices = self.step_indices().collect::<Vec<_>>();

        for index in step_indices {
            let step = self.get_step_data_mut(index)?;
            let normalizer = SelectionSetNormalizer { supergraph };
            normalizer.normalize_step(step)?;
        }

        Ok(())
    }
}

struct SelectionSetNormalizer<'a> {
    supergraph: &'a SupergraphState,
}

impl SelectionSetNormalizer<'_> {
    fn normalize_step(
        &self,
        step: &mut FetchStepData<MultiTypeFetchStep>,
    ) -> Result<(), FetchGraphError> {
        let definition_names = step
            .output
            .iter_selections()
            .map(|(definition_name, _)| definition_name.clone())
            .collect::<Vec<_>>();

        for definition_name in definition_names {
            let Some(selection_set) = step.output.selections_for_definition_mut(&definition_name)
            else {
                continue;
            };

            self.normalize_selection_set(&definition_name, selection_set)?;
        }

        Ok(())
    }

    fn normalize_selection_set(
        &self,
        current_type_name: &str,
        selection_set: &mut SelectionSet,
    ) -> Result<(), FetchGraphError> {
        let mut normalized_items = Vec::with_capacity(selection_set.items.len());

        for item in std::mem::take(&mut selection_set.items) {
            match item {
                SelectionItem::Field(mut field) => {
                    let child_type_name = self.field_return_type_name(current_type_name, &field)?;
                    self.normalize_selection_set(child_type_name, &mut field.selections)?;
                    normalized_items.push(SelectionItem::Field(field));
                }
                SelectionItem::InlineFragment(mut fragment) => {
                    self.normalize_selection_set(
                        &fragment.type_condition,
                        &mut fragment.selections,
                    )?;

                    if fragment.type_condition == current_type_name
                        && fragment.skip_if.is_none()
                        && fragment.include_if.is_none()
                    {
                        normalized_items.extend(fragment.selections.items);
                    } else {
                        normalized_items.push(SelectionItem::InlineFragment(fragment));
                    }
                }
                SelectionItem::FragmentSpread(_) => {
                    return Err(FetchGraphError::Internal(
                        "fragment spreads should have been inlined before selection-set normalization"
                            .to_string(),
                    ));
                }
            }
        }

        selection_set.items = normalized_items;

        Ok(())
    }

    fn field_return_type_name(
        &self,
        parent_type_name: &str,
        field: &FieldSelection,
    ) -> Result<&str, FetchGraphError> {
        if field.name == "__typename" {
            return Ok("String");
        }

        let definition = self
            .supergraph
            .definitions
            .get(parent_type_name)
            .ok_or_else(|| {
                FetchGraphError::Internal(format!(
                    "No definition found for type: {parent_type_name}"
                ))
            })?;

        let field_definition = definition.fields().get(&field.name).ok_or_else(|| {
            FetchGraphError::Internal(format!(
                "No field found for name '{}' in type '{}'",
                field.name, parent_type_name
            ))
        })?;

        Ok(field_definition.field_type.inner_type())
    }
}
