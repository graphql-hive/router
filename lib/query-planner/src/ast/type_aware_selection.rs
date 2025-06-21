use std::{fmt::Display, hash::Hash};

use crate::ast::{
    merge_path::Segment,
    safe_merge::{AliasesRecords, SafeSelectionSetMerger},
};

use super::{merge_path::MergePath, selection_item::SelectionItem, selection_set::SelectionSet};

#[derive(Debug, Clone)]
pub struct TypeAwareSelection {
    pub type_name: String,
    pub selection_set: SelectionSet,
}

impl Hash for TypeAwareSelection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_name.hash(state);
        self.selection_set.hash(state);
    }
}

impl Display for TypeAwareSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.selection_set.is_empty() {
            write!(f, "{{}}")
        } else {
            write!(f, "{}", self.selection_set)
        }
    }
}

impl PartialEq for TypeAwareSelection {
    fn eq(&self, other: &Self) -> bool {
        // TODO: This needs to be improved and check the internal selection sets correctly.
        self.type_name == other.type_name && self.selection_set.items == other.selection_set.items
    }
}

impl Eq for TypeAwareSelection {}

impl TypeAwareSelection {
    pub fn new(type_name: String, selection_set: SelectionSet) -> Self {
        Self {
            type_name,
            selection_set,
        }
    }

    pub fn cost(&self) -> u64 {
        let mut cost = 1;

        for node in &self.selection_set.items {
            cost += node.cost();
        }

        cost
    }

    pub fn contains(&self, other: &Self) -> bool {
        if self.type_name != other.type_name {
            return false;
        }

        selection_items_are_subset_of(&self.selection_set.items, &other.selection_set.items)
    }

    pub fn add(&mut self, to_add: &Self) {
        merge_selection_set(&mut self.selection_set, &to_add.selection_set, false);
    }

    pub fn add_at_path(&mut self, to_add: &Self, add_at_fetch_path: MergePath, as_first: bool) {
        if let Some(source) =
            find_selection_set_by_path_mut(&mut self.selection_set, &add_at_fetch_path)
        {
            merge_selection_set(source, &to_add.selection_set, as_first);
        }
    }

    pub fn add_at_path_and_solve_conflicts(
        &mut self,
        to_add: &Self,
        add_at_fetch_path: MergePath,
        (self_used_for_requires, other_used_for_requires): (bool, bool),
        as_first: bool,
    ) -> Option<AliasesRecords> {
        if let Some(source) =
            find_selection_set_by_path_mut(&mut self.selection_set, &add_at_fetch_path)
        {
            let mut merger = SafeSelectionSetMerger::default();
            let aliases_made = merger.merge_selection_set(
                source,
                &to_add.selection_set,
                (self_used_for_requires, other_used_for_requires),
                as_first,
            );

            if !aliases_made.is_empty() {
                return Some(aliases_made);
            }
        }

        None
    }

    pub fn has_typename_at_path(&self, lookup_path: &MergePath) -> bool {
        find_selection_set_by_path(
            &self.selection_set,
            &lookup_path.push(Segment::Field("__typename".to_string(), 0)),
        )
        .is_some()
    }
}

fn selection_item_is_subset_of(source: &SelectionItem, target: &SelectionItem) -> bool {
    match (source, target) {
        (SelectionItem::Field(source_field), SelectionItem::Field(target_field)) => {
            if source_field.name != target_field.name {
                return false;
            }

            if source_field.is_leaf() != target_field.is_leaf() {
                return false;
            }

            selection_items_are_subset_of(
                &source_field.selections.items,
                &target_field.selections.items,
            )
        }
        // TODO: support fragments
        _ => false,
    }
}

fn selection_items_are_subset_of(source: &[SelectionItem], target: &[SelectionItem]) -> bool {
    target.iter().all(|target_node| {
        source
            .iter()
            .any(|source_node| selection_item_is_subset_of(source_node, target_node))
    })
}

fn merge_selection_set(target: &mut SelectionSet, source: &SelectionSet, as_first: bool) {
    if source.items.is_empty() {
        return;
    }

    let mut pending_items = Vec::with_capacity(source.items.len());
    for source_item in source.items.iter() {
        let mut found = false;
        for target_item in target.items.iter_mut() {
            match (source_item, target_item) {
                (SelectionItem::Field(source_field), SelectionItem::Field(target_field)) => {
                    if source_field == target_field {
                        found = true;
                        merge_selection_set(
                            &mut target_field.selections,
                            &source_field.selections,
                            as_first,
                        );
                        break;
                    }
                }
                (
                    SelectionItem::InlineFragment(source_fragment),
                    SelectionItem::InlineFragment(target_fragment),
                ) => {
                    if source_fragment.type_condition == target_fragment.type_condition {
                        found = true;
                        merge_selection_set(
                            &mut target_fragment.selections,
                            &source_fragment.selections,
                            as_first,
                        );
                        break;
                    }
                }
                _ => {}
            }
        }

        if !found {
            pending_items.push(source_item.clone())
        }
    }

    if !pending_items.is_empty() {
        if as_first {
            let mut new_items = pending_items;
            new_items.append(&mut target.items);
            target.items = new_items;
        } else {
            target.items.extend(pending_items);
        }
    }
}

pub fn find_selection_set_by_path<'a>(
    root_selection_set: &'a SelectionSet,
    path: &MergePath,
) -> Option<&'a SelectionSet> {
    let mut current_selection_set = root_selection_set;

    for path_element in path.inner.iter() {
        match path_element {
            Segment::List => {
                continue;
            }
            Segment::Cast(type_name) => {
                let next_selection_set_option =
                    current_selection_set
                        .items
                        .iter()
                        .find_map(|item| match item {
                            SelectionItem::Field(_) => None,
                            SelectionItem::InlineFragment(f) => {
                                if f.type_condition.eq(type_name) {
                                    Some(&f.selections)
                                } else {
                                    None
                                }
                            }
                        });

                match next_selection_set_option {
                    Some(next_set) => {
                        current_selection_set = next_set;
                    }
                    None => {
                        return None;
                    }
                }
            }
            Segment::Field(field_name, args_hash) => {
                let next_selection_set_option =
                    current_selection_set
                        .items
                        .iter()
                        .find_map(|item| match item {
                            SelectionItem::Field(field) => {
                                if &field.name == field_name && field.arguments_hash() == *args_hash
                                {
                                    Some(&field.selections)
                                } else {
                                    None
                                }
                            }
                            SelectionItem::InlineFragment(..) => None,
                        });

                match next_selection_set_option {
                    Some(next_set) => {
                        current_selection_set = next_set;
                    }
                    None => {
                        return None;
                    }
                }
            }
        }
    }

    Some(current_selection_set)
}

pub fn find_selection_set_by_path_mut<'a>(
    root_selection_set: &'a mut SelectionSet,
    path: &MergePath,
) -> Option<&'a mut SelectionSet> {
    let mut current_selection_set = root_selection_set;

    for path_element in path.inner.iter() {
        match path_element {
            Segment::List => {
                continue;
            }
            Segment::Cast(type_name) => {
                let next_selection_set_option =
                    current_selection_set
                        .items
                        .iter_mut()
                        .find_map(|item| match item {
                            SelectionItem::Field(_) => None,
                            SelectionItem::InlineFragment(f) => {
                                if f.type_condition.eq(type_name) {
                                    Some(&mut f.selections)
                                } else {
                                    None
                                }
                            }
                        });

                match next_selection_set_option {
                    Some(next_set) => {
                        current_selection_set = next_set;
                    }
                    None => {
                        return None;
                    }
                }
            }
            Segment::Field(field_name, args_hash) => {
                let next_selection_set_option =
                    current_selection_set
                        .items
                        .iter_mut()
                        .find_map(|item| match item {
                            SelectionItem::Field(field) => {
                                if field.selection_identifier() == field_name
                                    && field.arguments_hash() == *args_hash
                                {
                                    Some(&mut field.selections)
                                } else {
                                    None
                                }
                            }
                            SelectionItem::InlineFragment(..) => None,
                        });

                match next_selection_set_option {
                    Some(next_set) => {
                        current_selection_set = next_set;
                    }
                    None => {
                        return None;
                    }
                }
            }
        }
    }
    Some(current_selection_set)
}

/// Find the arguments conflicts between two selections.
/// Returns a vector of tuples containing the indices of conflicting fields in both "source" and "other"
/// Both indices are returned in order to allow for easy resolution of conflicts later, in either side.
pub fn find_arguments_conflicts(
    source: &TypeAwareSelection,
    other: &TypeAwareSelection,
) -> Vec<(usize, usize)> {
    other
        .selection_set
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, other_selection)| {
            if let SelectionItem::Field(other_field) = other_selection {
                let other_identifier = other_field.selection_identifier();
                let other_args_hash = other_field.arguments_hash();

                let existing_in_self = source.selection_set.items.iter().enumerate().find_map(
                    |(self_index, self_selection)| {
                        if let SelectionItem::Field(self_field) = self_selection {
                            // If the field selection identifier matches and the arguments hash is different,
                            // then it means that we can't merge the two input siblings
                            if self_field.selection_identifier() == other_identifier
                                && self_field.arguments_hash() != other_args_hash
                            {
                                return Some(self_index);
                            }
                        }

                        None
                    },
                );

                if let Some(existing_index) = existing_in_self {
                    return Some((existing_index, index));
                }

                return None;
            }

            None
        })
        .collect()
}
