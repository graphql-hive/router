use tracing::instrument;

use crate::{
    ast::{
        selection_item::SelectionItem,
        selection_set::{merge_selection_set, SelectionSet},
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
        for (definition_name, selection_set) in step.output.iter_selections_mut() {
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
                    let child_type_name = self
                        .supergraph
                        .field_return_type_name(current_type_name, field.name.as_str())
                        .ok_or_else(|| {
                            FetchGraphError::Internal(format!(
                                "No field found for name '{}' in type '{}'",
                                field.name, current_type_name
                            ))
                        })?;
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
                        // normalized_items.extend(fragment.selections.items);
                        let mut merged = SelectionSet {
                            items: std::mem::take(&mut normalized_items),
                        };
                        merge_selection_set(&mut merged, &fragment.selections, false);
                        normalized_items = merged.items;
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
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::query::{Definition, OperationDefinition};

    use crate::{
        state::supergraph_state::SupergraphState,
        utils::parsing::{parse_operation, parse_schema},
    };

    use super::SelectionSetNormalizer;

    #[test]
    fn ensure_deduplication() {
        let schema = parse_schema(
            r#"
            directive @join__type(graph: join__Graph!, key: join__FieldSet) repeatable on OBJECT
            scalar join__FieldSet
            enum join__Graph { A @join__graph(name: "a", url: "") }
            type Query @join__type(graph: A) { user: User }
            type User @join__type(graph: A) { id: ID! name: String! }
          "#,
        );
        let supergraph = SupergraphState::new(&schema);
        let normalizer = SelectionSetNormalizer {
            supergraph: &supergraph,
        };

        let op = parse_operation("{ id ... on User { id } }");
        let mut selection_set = match op.definitions.first() {
            Some(Definition::Operation(OperationDefinition::SelectionSet(s))) => s.clone().into(),
            _ => panic!("expected top-level selection set"),
        };

        normalizer
            .normalize_selection_set("User", &mut selection_set)
            .unwrap();

        insta::assert_snapshot!(&selection_set.to_string(), @"{id}");
    }
}
