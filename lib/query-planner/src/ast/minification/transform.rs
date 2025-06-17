use std::collections::HashMap;
use std::mem::take;

use crate::ast::document::Document;
use crate::ast::fragment::FragmentDefinition;
use crate::ast::minification::error::MinificationError;
use crate::ast::minification::selection_id::generate_selection_id;
use crate::ast::minification::stats::Stats;
use crate::ast::operation::OperationDefinition;
use crate::ast::selection_item::SelectionItem;
use crate::ast::selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet};
use crate::state::supergraph_state::SupergraphState;

type Fragments = HashMap<u64, FragmentDefinition>;

pub fn transform_operation(
    supergraph: &SupergraphState,
    stats: Stats,
    root_type_name: &str,
    operation: OperationDefinition,
) -> Result<Document, MinificationError> {
    if !stats.has_duplicates() {
        return Ok(Document {
            operation,
            fragments: vec![],
        });
    }

    let mut fragments = Fragments::new();
    let mut next_fragment_name_idx = 0;
    let mut operation = operation;

    let new_selection_set = transform_selection_set(
        supergraph,
        &stats,
        &mut fragments,
        &mut next_fragment_name_idx,
        &mut operation.selection_set,
        root_type_name,
    )?;

    let mut fragments_vec: Vec<FragmentDefinition> = fragments.into_values().collect();
    fragments_vec.sort();

    Ok(Document {
        operation: OperationDefinition {
            name: operation.name,
            operation_kind: operation.operation_kind,
            selection_set: new_selection_set,
            variable_definitions: operation.variable_definitions,
        },
        fragments: fragments_vec,
    })
}

fn transform_selection_set(
    supergraph: &SupergraphState,
    stats: &Stats,
    fragments: &mut Fragments,
    next_fragment_name_idx: &mut usize,
    selection_set: &mut SelectionSet,
    type_name: &str,
) -> Result<SelectionSet, MinificationError> {
    let id = generate_selection_id(type_name, selection_set);
    if stats.is_duplicated(&id) {
        // This is a duplicate, so replace it with a fragment spread
        let fragment_name = get_or_create_fragment(
            supergraph,
            stats,
            fragments,
            next_fragment_name_idx,
            &id,
            type_name,
            selection_set,
        )?;
        Ok(SelectionSet {
            items: vec![SelectionItem::FragmentSpread(fragment_name)],
        })
    } else {
        let new_items = transform_selection_set_items(
            supergraph,
            stats,
            fragments,
            next_fragment_name_idx,
            selection_set,
            type_name,
        )?;

        Ok(SelectionSet { items: new_items })
    }
}

fn transform_selection_set_items(
    supergraph: &SupergraphState,
    stats: &Stats,
    fragments: &mut Fragments,
    next_fragment_name_idx: &mut usize,
    selection_set: &mut SelectionSet,
    type_name: &str,
) -> Result<Vec<SelectionItem>, MinificationError> {
    let mut new_items: Vec<SelectionItem> = Vec::with_capacity(selection_set.items.len());

    for item in &mut selection_set.items {
        match item {
            SelectionItem::Field(field) => {
                new_items.push(transform_field(
                    supergraph,
                    stats,
                    fragments,
                    next_fragment_name_idx,
                    field,
                    type_name,
                )?);
            }
            SelectionItem::InlineFragment(fragment) => {
                new_items.push(transform_inline_fragment(
                    supergraph,
                    stats,
                    fragments,
                    next_fragment_name_idx,
                    fragment,
                )?);
            }
            SelectionItem::FragmentSpread(spread) => {
                // pass it through
                new_items.push(SelectionItem::FragmentSpread(take(spread)));
            }
        }
    }

    Ok(new_items)
}

fn transform_inline_fragment(
    supergraph: &SupergraphState,
    stats: &Stats,
    fragments: &mut Fragments,
    next_fragment_name_idx: &mut usize,
    fragment: &mut InlineFragmentSelection,
) -> Result<SelectionItem, MinificationError> {
    let new_selections = transform_selection_set(
        supergraph,
        stats,
        fragments,
        next_fragment_name_idx,
        &mut fragment.selections,
        &fragment.type_condition,
    )?;

    Ok(SelectionItem::InlineFragment(InlineFragmentSelection {
        type_condition: fragment.type_condition.clone(),
        selections: new_selections,
    }))
}

fn transform_field(
    supergraph: &SupergraphState,
    stats: &Stats,
    fragments: &mut Fragments,
    next_fragment_name_idx: &mut usize,
    field: &mut FieldSelection,
    type_name: &str,
) -> Result<SelectionItem, MinificationError> {
    if field.is_introspection_field() {
        return Ok(SelectionItem::Field(field.clone()));
    }

    // Special case where _entities is used,
    // and as you know, Query._entities is not part of the schema,
    // so we need to handle it specially.
    // The content of _entities is a list of inline fragments.
    // We can extract the type condition and pass it as the type of the selection set.
    // This way we don't lookup the type in the schema.
    if field.name == "_entities" {
        let mut new_entities: Vec<SelectionItem> = Vec::with_capacity(field.selections.items.len());

        for item in &mut field.selections.items {
            match item {
                SelectionItem::Field(field) => {
                    if field.is_introspection_field() {
                        new_entities.push(SelectionItem::Field(take(field)));
                        continue;
                    }

                    return Err(MinificationError::UnsupportedFieldInEntities(
                        field.name.clone(),
                    ));
                }
                SelectionItem::InlineFragment(fragment) => {
                    let new_selections = transform_selection_set(
                        supergraph,
                        stats,
                        fragments,
                        next_fragment_name_idx,
                        &mut fragment.selections,
                        &fragment.type_condition,
                    )?;

                    new_entities.push(SelectionItem::InlineFragment(InlineFragmentSelection {
                        type_condition: fragment.type_condition.clone(),
                        selections: new_selections,
                    }));
                }
                SelectionItem::FragmentSpread(spread) => {
                    new_entities.push(SelectionItem::FragmentSpread(take(spread)));
                }
            }
        }

        return Ok(SelectionItem::Field(FieldSelection {
            name: take(&mut field.name),
            alias: field.alias.take(),
            arguments: field.arguments.take(),
            selections: SelectionSet {
                items: new_entities,
            },
            skip_if: field.skip_if.take(),
            include_if: field.include_if.take(),
        }));
    }

    let child_type_name = supergraph
        .definitions
        .get(type_name)
        .and_then(|t| t.fields().get(&field.name))
        .ok_or_else(|| MinificationError::FieldNotFound(field.name.clone(), type_name.to_string()))?
        .field_type
        .inner_type();
    let new_selections = transform_selection_set(
        supergraph,
        stats,
        fragments,
        next_fragment_name_idx,
        &mut field.selections,
        child_type_name,
    )?;
    Ok(SelectionItem::Field(FieldSelection {
        alias: field.alias.take(),
        name: take(&mut field.name),
        arguments: field.arguments.take(),
        selections: new_selections,
        skip_if: field.skip_if.take(),
        include_if: field.include_if.take(),
    }))
}

pub fn get_or_create_fragment(
    supergraph: &SupergraphState,
    stats: &Stats,
    fragments: &mut Fragments,
    next_fragment_name_idx: &mut usize,
    id: &u64,
    type_name: &str,
    selection_set: &mut SelectionSet,
) -> Result<String, MinificationError> {
    if let Some(existing_frag) = fragments.get(id) {
        return Ok(existing_frag.name.clone());
    }

    let fragment_name = generate_fragment_name(next_fragment_name_idx);
    let placeholder = FragmentDefinition {
        name: fragment_name.clone(),
        type_condition: type_name.to_string(),
        selection_set: SelectionSet::default(),
    };
    fragments.insert(*id, placeholder);

    let new_items = transform_selection_set_items(
        supergraph,
        stats,
        fragments,
        next_fragment_name_idx,
        selection_set,
        type_name,
    )?;

    // Replace the placeholder with the final fragment
    fragments.insert(
        *id,
        FragmentDefinition {
            name: fragment_name.clone(),
            type_condition: type_name.to_string(),
            selection_set: SelectionSet { items: new_items },
        },
    );

    Ok(fragment_name)
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
    use crate::ast::minification::transform::{generate_fragment_name, ALPHABET};

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
