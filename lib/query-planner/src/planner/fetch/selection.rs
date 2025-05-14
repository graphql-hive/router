use crate::planner::walker::selection::{SelectionItem, SelectionSet};
use std::fmt::{self, Debug, Display};

#[derive(Debug, Clone)]
pub struct Selection {
    pub selection_set: SelectionSet,
    pub type_name: String,
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
    path: &[String],
) -> Option<&'a mut SelectionSet> {
    let mut current_selection_set = root_selection_set;

    for path_element in path.iter() {
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

impl Selection {
    // TODO try to create a trait called SubsetOf or something to mimic PartialEq
    pub fn contains(&self, other: &Selection) -> bool {
        if self.type_name != other.type_name {
            return false;
        }

        return selection_items_are_subset_of(
            &self.selection_set.items,
            &other.selection_set.items,
        );
    }

    pub fn add(&mut self, to_add: Selection) {
        merge_selection_set(&mut self.selection_set, &to_add.selection_set, false);
    }

    pub fn add_at_path(&mut self, to_add: &Selection, add_at_path: Vec<String>, as_first: bool) {
        if let Some(source) = find_selection_set_by_path_mut(&mut self.selection_set, &add_at_path)
        {
            merge_selection_set(source, &to_add.selection_set, as_first);
        }
    }
}

impl PartialEq for Selection {
    fn eq(&self, other: &Selection) -> bool {
        if self.type_name != other.type_name {
            return false;
        }

        return self.selection_set.eq(&other.selection_set);
    }
}

impl Display for Selection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.selection_set)
    }
}
