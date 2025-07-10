use std::{
    collections::{BTreeSet, HashMap},
    fmt::Display,
};

use crate::ast::{
    merge_path::MergePath,
    selection_item::SelectionItem,
    selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    type_aware_selection::{find_selection_set_by_path_mut, merge_selection_set},
};

#[derive(Debug, Clone, Default)]
pub struct FetchStepSelections {
    selections: HashMap<String, SelectionSet>,
}

impl FetchStepSelections {
    pub fn new() -> Self {
        FetchStepSelections {
            selections: HashMap::new(),
        }
    }
}

impl Display for FetchStepSelections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_selection_set: SelectionSet = self.into();

        write!(f, "{}", as_selection_set)
    }
}

impl From<&FetchStepSelections> for SelectionSet {
    fn from(value: &FetchStepSelections) -> Self {
        if let Some(as_root) = value.as_root_selection() {
            as_root.1.clone()
        } else {
            SelectionSet {
                items: value
                    .iter()
                    .map(|(def_name, selections)| {
                        SelectionItem::InlineFragment(InlineFragmentSelection {
                            type_condition: def_name.clone(),
                            include_if: None,
                            skip_if: None,
                            selections: selections.clone(),
                        })
                    })
                    .collect(),
            }
        }
    }
}

impl FetchStepSelections {
    pub fn iter(&self) -> impl Iterator<Item = (&String, &SelectionSet)> {
        self.selections.iter()
    }

    pub fn as_root_selection(&self) -> Option<(&String, &SelectionSet)> {
        if self.selections.len() == 1 {
            let (type_name, selection_set) = self.selections.iter().next().unwrap();

            if type_name == "Query" || type_name == "Mutation" || type_name == "Subscription" {
                return Some((type_name, selection_set));
            } else {
                return None;
            }
        }

        None
    }

    pub fn selections_for_definition(&mut self, definition_name: &str) -> &mut SelectionSet {
        self.selections
            .entry(definition_name.to_string())
            .or_insert_with(SelectionSet::default)
    }

    pub fn is_selecting_definition(&self, definition_name: &str) -> bool {
        self.selections.contains_key(definition_name)
    }

    pub fn variable_usages(&self) -> BTreeSet<String> {
        let mut usages = BTreeSet::new();

        for selection_set in self.selections.values() {
            usages.extend(selection_set.variable_usages());
        }

        usages
    }

    pub fn add_at_path(
        &mut self,
        definition_name: &str,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
    ) {
        self.add_at_path_inner(definition_name, fetch_path, selection_set, false);
    }

    pub fn add_selection_typename(&mut self, definition_name: &str, fetch_path: &MergePath) {
        self.add_at_path_inner(
            definition_name,
            fetch_path,
            SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection::new_typename())],
            },
            true,
        );
    }

    fn add_at_path_inner(
        &mut self,
        definition_name: &str,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
        as_first: bool,
    ) {
        let mut current = self.selections_for_definition(definition_name);

        if let Some(selection_at_path) = find_selection_set_by_path_mut(&mut current, &fetch_path) {
            merge_selection_set(selection_at_path, &selection_set, as_first);
        } else {
            panic!(
                "Path '{}' cannot be found in selection set: '{}'",
                fetch_path, current
            );
        }
    }
}
