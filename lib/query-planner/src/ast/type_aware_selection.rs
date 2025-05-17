use std::{fmt::Display, hash::Hash};

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

        return selection_items_are_subset_of(
            &self.selection_set.items,
            &other.selection_set.items,
        );
    }

    pub fn add(&mut self, to_add: Self) {
        merge_selection_set(&mut self.selection_set, &to_add.selection_set, false);
    }

    pub fn add_at_path(&mut self, to_add: &Self, add_at_path: MergePath, as_first: bool) {
        if let Some(source) = find_selection_set_by_path_mut(&mut self.selection_set, add_at_path) {
            merge_selection_set(source, &to_add.selection_set, as_first);
        }
    }
}

fn selection_item_is_subset_of(source: &SelectionItem, target: &SelectionItem) -> bool {
    return match (source, target) {
        (SelectionItem::Field(source_field), SelectionItem::Field(target_field)) => {
            if source_field.name != target_field.name {
                return false;
            }

            if source_field.is_leaf != target_field.is_leaf {
                return false;
            }

            return selection_items_are_subset_of(
                &source_field.selections.items,
                &target_field.selections.items,
            );
        }
        // TODO: support fragments
        _ => false,
    };
}

fn selection_items_are_subset_of(source: &Vec<SelectionItem>, target: &Vec<SelectionItem>) -> bool {
    return target.iter().all(|target_node| {
        source
            .iter()
            .any(|source_node| selection_item_is_subset_of(source_node, target_node))
    });
}

fn merge_selection_set(target: &mut SelectionSet, source: &SelectionSet, as_first: bool) {
    if source.items.is_empty() {
        return;
    }

    let mut pending_items = Vec::with_capacity(source.items.len());

    source.items.iter().for_each(|source_item| {
        let matching_target_item = target.items.iter_mut().find(|target_item| {
            matches!(
                (target_item, source_item),
                (SelectionItem::Field(target_field), SelectionItem::Field(source_field))
                if target_field.name == source_field.name
            )
        });

        match matching_target_item {
            Some(target_item) => {
                if let SelectionItem::Field(target_field) = target_item {
                    merge_selection_set(&mut target_field.selections, source, as_first);
                }
            }
            None => pending_items.push(source_item.clone()),
        }
    });

    if !pending_items.is_empty() {
        if as_first {
            let mut new_items = pending_items;
            new_items.extend(target.items.drain(..));
            target.items = new_items;
        } else {
            target.items.extend(pending_items);
        }
    }
}

fn find_selection_set_by_path_mut<'a>(
    root_selection_set: &'a mut SelectionSet,
    path: MergePath,
) -> Option<&'a mut SelectionSet> {
    let mut current_selection_set = root_selection_set;

    for path_element in path.inner.iter() {
        if path_element == "@" {
            continue;
        }

        let next_selection_set_option =
            current_selection_set
                .items
                .iter_mut()
                .find_map(|item| match item {
                    SelectionItem::Field(field) => {
                        if field.name.eq(path_element) {
                            Some(&mut field.selections)
                        } else {
                            None
                        }
                    }
                    SelectionItem::Fragment(..) => None,
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

    Some(current_selection_set)
}
