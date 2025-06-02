use std::{fmt::Display, hash::Hash};

use crate::ast::merge_path::Segment;

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
            find_selection_set_by_path_mut(&mut self.selection_set, add_at_fetch_path)
        {
            merge_selection_set(source, &to_add.selection_set, as_first);
        }
    }

    pub fn has_typename_at_path(&self, lookup_path: &MergePath) -> bool {
        find_selection_set_by_path(
            &self.selection_set,
            &lookup_path.push(Segment::Field("__typename".to_string())),
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
            if let (SelectionItem::Field(source_field), SelectionItem::Field(target_field)) =
                (source_item, target_item)
            {
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

fn find_selection_set_by_path<'a>(
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
            Segment::Field(field_name) => {
                let next_selection_set_option =
                    current_selection_set
                        .items
                        .iter()
                        .find_map(|item| match item {
                            SelectionItem::Field(field) => {
                                if field.name.eq(field_name) {
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

fn find_selection_set_by_path_mut(
    root_selection_set: &mut SelectionSet,
    path: MergePath,
) -> Option<&mut SelectionSet> {
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
            Segment::Field(field_name) => {
                let next_selection_set_option =
                    current_selection_set
                        .items
                        .iter_mut()
                        .find_map(|item| match item {
                            SelectionItem::Field(field) => {
                                if field.name.eq(field_name) {
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
