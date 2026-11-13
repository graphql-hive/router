use ahash::{HashMap, HashMapExt};

use crate::query_planner::ast::flat::arena::{Arena, ListId, NodeId};
use crate::query_planner::ast::flat::plan_document::{PlanDocument, PlanFragment};
use crate::query_planner::ast::flat::Kind;
use crate::query_planner::ast::minification::error::MinificationError;
use crate::query_planner::ast::minification::selection_id::{generate_selection_id, SelectionId};
use crate::query_planner::ast::minification::stats::Stats;
use crate::query_planner::state::supergraph_state::SupergraphState;

/// Swaps every repeated selection set for a spread of a fragment, in place. Every list of
/// `document` has to be its own, nothing shared.
pub fn transform_operation(
    supergraph: &SupergraphState,
    stats: Stats,
    root_type_name: &str,
    document: &mut PlanDocument,
) -> Result<(), MinificationError> {
    if !stats.has_duplicates() {
        return Ok(());
    }

    let mut transform = Transform {
        supergraph,
        stats: &stats,
        by_id: HashMap::new(),
        fragments: vec![],
        next_fragment_name_idx: 0,
    };
    transform.selection_set(&mut document.arena, document.selections, root_type_name)?;

    let mut fragments = transform.fragments;
    fragments.sort_by(|a, b| a.name.cmp(&b.name));
    document.fragments.extend(fragments);
    Ok(())
}

struct Transform<'a> {
    supergraph: &'a SupergraphState,
    stats: &'a Stats,
    /// Into `fragments`
    by_id: HashMap<SelectionId, usize>,
    fragments: Vec<PlanFragment>,
    next_fragment_name_idx: usize,
}

impl Transform<'_> {
    fn selection_set(
        &mut self,
        arena: &mut Arena,
        list: ListId,
        type_name: &str,
    ) -> Result<(), MinificationError> {
        let id = generate_selection_id(arena, type_name, list);
        if self.stats.is_duplicated(&id) {
            let fragment_name = self.get_or_create_fragment(arena, id, list, type_name)?;
            let spread = arena.new_spread(&fragment_name);
            arena.set_items(list, &[spread]);
            Ok(())
        } else {
            self.items(arena, list, type_name)
        }
    }

    fn items(
        &mut self,
        arena: &mut Arena,
        list: ListId,
        type_name: &str,
    ) -> Result<(), MinificationError> {
        // what's below changes, `list` itself doesn't
        for index in 0..arena.items(list).len() {
            let id = arena.items(list)[index];
            let node = *arena.node(id);
            match node.kind {
                Kind::Field => self.field(arena, id, type_name)?,
                Kind::InlineFragment => {
                    let type_condition = arena.str(node.name).to_string();
                    self.selection_set(arena, node.children, &type_condition)?;
                }
                Kind::FragmentSpread => {}
            }
        }
        Ok(())
    }

    fn field(
        &mut self,
        arena: &mut Arena,
        id: NodeId,
        type_name: &str,
    ) -> Result<(), MinificationError> {
        let node = *arena.node(id);
        let name = arena.str(node.name);
        if name.starts_with("__") {
            return Ok(());
        }

        // `Query._entities` isn't in the schema. What's under it is an inline fragment per
        // entity type, so their type conditions give the types.
        if name == "_entities" {
            for index in 0..arena.items(node.children).len() {
                let entity = *arena.node(arena.items(node.children)[index]);
                match entity.kind {
                    Kind::Field => {
                        let entity_field = arena.str(entity.name);
                        if !entity_field.starts_with("__") {
                            return Err(MinificationError::UnsupportedFieldInEntities(
                                type_name.to_string(),
                                entity_field.to_string(),
                            ));
                        }
                    }
                    Kind::InlineFragment => {
                        let type_condition = arena.str(entity.name).to_string();
                        self.selection_set(arena, entity.children, &type_condition)?;
                    }
                    Kind::FragmentSpread => {}
                }
            }
            return Ok(());
        }

        let supergraph = self.supergraph;
        let child_type_name = supergraph
            .definitions
            .get(type_name)
            .and_then(|t| t.fields().get(name))
            .ok_or_else(|| {
                MinificationError::FieldNotFound(name.to_string(), type_name.to_string())
            })?
            .field_type
            .inner_type();
        self.selection_set(arena, node.children, child_type_name)
    }

    /// The fragment's name. A new fragment takes the items of `list`, minified.
    fn get_or_create_fragment(
        &mut self,
        arena: &mut Arena,
        id: SelectionId,
        list: ListId,
        type_name: &str,
    ) -> Result<String, MinificationError> {
        if let Some(&index) = self.by_id.get(&id) {
            return Ok(self.fragments[index].name.clone());
        }

        let name = generate_fragment_name(&mut self.next_fragment_name_idx);
        let selections = arena.new_list();
        // in before what's below it, so the names go in the order the tree walk meets them
        self.by_id.insert(id, self.fragments.len());
        self.fragments.push(PlanFragment {
            name: name.clone(),
            type_condition: type_name.to_string(),
            selections,
        });

        self.items(arena, list, type_name)?;
        // a list of its own: `list` becomes the spread right after
        let items = arena.items(list).to_vec();
        arena.set_items(selections, &items);

        Ok(name)
    }
}

pub const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
pub fn generate_fragment_name(index: &mut usize) -> String {
    let mut i = *index;
    *index += 1;
    let mut name = String::new();

    if i < ALPHABET.len() {
        name.push(ALPHABET[i] as char);
        return name;
    }

    while i > 0 {
        name.push(ALPHABET[i % ALPHABET.len()] as char);
        i /= ALPHABET.len();
    }

    name
}

#[cfg(test)]
mod tests {
    use crate::query_planner::ast::minification::transform::{generate_fragment_name, ALPHABET};

    #[test]
    fn generate_fragment_name_test() {
        let alphabet_len = ALPHABET.len();
        let mut index;

        // 1 char
        index = 0;
        assert_eq!(generate_fragment_name(&mut index), "a", "first 1-char");
        index = 1;
        assert_eq!(generate_fragment_name(&mut index), "b");
        index = 26;
        assert_eq!(generate_fragment_name(&mut index), "A");
        index = alphabet_len - 1;
        assert_eq!(generate_fragment_name(&mut index), "Z", "last 1-char");

        // 2 chars
        index = alphabet_len;
        assert_eq!(generate_fragment_name(&mut index), "ab", "first 2-char");
        index = alphabet_len + 1;
        assert_eq!(generate_fragment_name(&mut index), "bb");
        index = alphabet_len * 2;
        assert_eq!(generate_fragment_name(&mut index), "ac");
        index = alphabet_len.pow(2) - 1;
        assert_eq!(generate_fragment_name(&mut index), "ZZ", "last 2-char");

        // 3 chars
        index = alphabet_len.pow(2);
        assert_eq!(generate_fragment_name(&mut index), "aab", "first 3-char");
        index = alphabet_len.pow(2) + 1;
        assert_eq!(generate_fragment_name(&mut index), "bab");
        index = alphabet_len.pow(2) * 2;
        assert_eq!(generate_fragment_name(&mut index), "aac");
        index = alphabet_len.pow(3) - 1;
        assert_eq!(generate_fragment_name(&mut index), "ZZZ", "last 3-char");
    }
}
