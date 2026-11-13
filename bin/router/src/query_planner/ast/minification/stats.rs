use ahash::{HashMap, HashMapExt};

use crate::query_planner::ast::flat::arena::{Arena, ListId};
use crate::query_planner::ast::flat::Kind;
use crate::query_planner::ast::minification::error::MinificationError;
use crate::query_planner::ast::minification::selection_id::{generate_selection_id, SelectionId};
use crate::query_planner::state::supergraph_state::SupergraphState;

pub struct Stats {
    state: HashMap<SelectionId, usize>,
    contains_duplicates: bool,
}

impl Stats {
    pub fn from_operation(
        arena: &Arena,
        list: ListId,
        supergraph: &SupergraphState,
        root_type_name: &str,
    ) -> Result<Self, MinificationError> {
        let mut stats = Self {
            state: HashMap::new(),
            contains_duplicates: false,
        };

        walk_and_collect_stats(&mut stats, supergraph, arena, list, root_type_name)?;
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
    arena: &Arena,
    list: ListId,
    type_name: &str,
) -> Result<(), MinificationError> {
    if arena.is_empty(list) {
        return Ok(());
    }

    let id = generate_selection_id(arena, type_name, list);
    let occurrences = stats.increase(id);

    if *occurrences > 1 {
        return Ok(());
    }

    let type_def = supergraph
        .definitions
        .get(type_name)
        .ok_or_else(|| MinificationError::TypeNotFound(type_name.to_string()))?;

    for &item in arena.items(list) {
        let node = arena.node(item);
        match node.kind {
            Kind::Field => {
                let name = arena.str(node.name);
                if name.starts_with("__") {
                    continue;
                }

                // `Query._entities` isn't in the schema. What's under it is an inline fragment
                // per entity type, so their type conditions give the types.
                if name == "_entities" {
                    for &entity in arena.items(node.children) {
                        let entity_node = arena.node(entity);
                        match entity_node.kind {
                            Kind::Field => {
                                let entity_field = arena.str(entity_node.name);
                                if entity_field.starts_with("__") {
                                    continue;
                                }

                                return Err(MinificationError::UnsupportedFieldInEntities(
                                    type_def.name().to_string(),
                                    entity_field.to_string(),
                                ));
                            }
                            Kind::InlineFragment => {
                                walk_and_collect_stats(
                                    stats,
                                    supergraph,
                                    arena,
                                    entity_node.children,
                                    arena.str(entity_node.name),
                                )?;
                            }
                            Kind::FragmentSpread => {
                                return Err(MinificationError::UnsupportedFragmentSpread);
                            }
                        }
                    }
                    continue;
                }

                let child_type_name = type_def
                    .fields()
                    .get(name)
                    .ok_or_else(|| {
                        MinificationError::FieldNotFound(
                            name.to_string(),
                            type_def.name().to_string(),
                        )
                    })?
                    .field_type
                    .inner_type();
                walk_and_collect_stats(stats, supergraph, arena, node.children, child_type_name)?;
            }
            Kind::InlineFragment => {
                walk_and_collect_stats(
                    stats,
                    supergraph,
                    arena,
                    node.children,
                    arena.str(node.name),
                )?;
            }
            Kind::FragmentSpread => {
                return Err(MinificationError::UnsupportedFragmentSpread);
            }
        }
    }

    Ok(())
}
