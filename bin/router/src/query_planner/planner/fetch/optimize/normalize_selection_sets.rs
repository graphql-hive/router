use tracing::instrument;

use crate::query_planner::{
    ast::flat::{
        arena::{Arena, ListId},
        Kind, NONE,
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
            let (step, arena) = self.step_and_arena_mut(index)?;
            let normalizer = SelectionSetNormalizer { supergraph };
            normalizer.normalize_step(arena, step)?;
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
        arena: &mut Arena,
        step: &mut FetchStepData<MultiTypeFetchStep>,
    ) -> Result<(), FetchGraphError> {
        for (definition_name, list) in step.output.iter_selections() {
            self.normalize_selection_set(arena, definition_name, list)?;
        }

        Ok(())
    }

    /// Puts the selections of `... on <current type>` without conditions into the set itself.
    fn normalize_selection_set(
        &self,
        arena: &mut Arena,
        current_type_name: &str,
        list: ListId,
    ) -> Result<(), FetchGraphError> {
        let items = arena.items(list).to_vec();
        let normalized = arena.new_list();

        for item in items {
            let node = *arena.node(item);
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
                    self.normalize_selection_set(arena, child_type_name, node.children)?;
                    arena.push(normalized, item);
                }
                Kind::InlineFragment => {
                    let type_condition = arena.str(node.name).to_string();
                    self.normalize_selection_set(arena, &type_condition, node.children)?;

                    if type_condition == current_type_name
                        && node.skip_if == NONE
                        && node.include_if == NONE
                    {
                        arena.merge(normalized, node.children, false)?;
                    } else {
                        arena.push(normalized, item);
                    }
                }
                Kind::FragmentSpread => {
                    return Err(FetchGraphError::Internal(
                        "fragment spreads should have been inlined before selection-set normalization"
                            .to_string(),
                    ));
                }
            }
        }

        let normalized = arena.items(normalized).to_vec();
        arena.set_items(list, &normalized);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::query::{Definition, OperationDefinition};

    use crate::query_planner::{
        state::supergraph_state::SupergraphState,
        utils::parsing::{parse_operation, parse_schema},
    };

    use super::SelectionSetNormalizer;
    use crate::query_planner::ast::{flat::arena::Arena, selection_set::SelectionSet};

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
        let selection_set: SelectionSet = match op.definitions.first() {
            Some(Definition::Operation(OperationDefinition::SelectionSet(s))) => s.clone().into(),
            _ => panic!("expected top-level selection set"),
        };
        let mut arena = Arena::new();
        let list = arena.import(&selection_set);

        normalizer
            .normalize_selection_set(&mut arena, "User", list)
            .unwrap();

        insta::assert_snapshot!(&arena.export(list).to_string(), @"{id}");
    }
}
