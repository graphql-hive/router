use std::{
    collections::{BTreeSet, HashMap},
    fmt::Display,
};

use tracing::trace;

use crate::ast::{
    merge_path::MergePath,
    safe_merge::{AliasesRecords, SafeSelectionSetMerger},
    selection_item::SelectionItem,
    selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    type_aware_selection::{
        find_selection_set_by_path_mut, merge_selection_set, TypeAwareSelection,
    },
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

    pub fn len(&self) -> usize {
        self.selections.len()
    }

    pub fn selection_definitions(&self) -> Vec<&str> {
        self.selections.keys().map(|key| key.as_str()).collect()
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
            // if value.len() == 1 {
            // return value.iter().next().unwrap().1.clone();
            // }

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

    pub fn maybe_root_type_name(&self) -> Option<&str> {
        self.as_root_selection()
            .map(|(type_name, _)| type_name.as_str())
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

    pub fn is_empty(&self) -> bool {
        self.selections.is_empty()
            || self
                .iter()
                .all(|(_, selection_set)| selection_set.is_empty())
    }

    pub fn selections_for_definition(&self, definition_name: &str) -> Option<&SelectionSet> {
        self.selections.get(definition_name)
    }

    pub fn selections_for_definition_mut(&mut self, definition_name: &str) -> &mut SelectionSet {
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

    pub fn add_from_another_at_path(&mut self, other: &Self, path: &MergePath) {
        for (def_name, selection_set) in other.iter() {
            self.add_at_path_inner(def_name, path, selection_set.clone(), false);
        }
    }

    pub fn migrate_from_another_at_path(&mut self, other: &Self, path: &MergePath) {
        let to_def_name = self
            .maybe_root_type_name()
            .map(|v| v.to_string())
            .expect("missing root");

        for (_, selection_set) in other.iter() {
            self.add_at_path_inner(&to_def_name, path, selection_set.clone(), false);
        }
    }

    pub fn safe_add_from_another_at_path(
        &mut self,
        other: &Self,
        fetch_path: &MergePath,
        (self_used_for_requires, other_used_for_requires): (bool, bool),
    ) -> Vec<(String, AliasesRecords)> {
        let mut aliases_made: Vec<(String, AliasesRecords)> = Vec::new();
        let maybe_root_def = self.maybe_root_type_name().map(|v| v.to_string());

        for (definition_name, selection_set) in other.iter() {
            let target_type_name = if fetch_path.is_empty() {
                definition_name
            } else {
                maybe_root_def.as_ref().unwrap_or(definition_name)
            };
            let mut current = self.selections_for_definition_mut(target_type_name);

            if let Some(selection_at_path) =
                find_selection_set_by_path_mut(&mut current, &fetch_path)
            {
                let mut merger = SafeSelectionSetMerger::default();
                let current_aliases_made = merger.merge_selection_set(
                    selection_at_path,
                    &selection_set,
                    (self_used_for_requires, other_used_for_requires),
                    false,
                );

                if !current_aliases_made.is_empty() {
                    aliases_made.push((target_type_name.to_string(), current_aliases_made));
                }
            } else {
                // TODO: Replace with error handling
                panic!(
                    "[{}]: Path '{}' cannot be found in selection set: '{}'",
                    target_type_name, fetch_path, current
                );
            }
        }

        aliases_made
    }

    pub fn add_at_root(&mut self, definition_name: &str, selection_set: SelectionSet) {
        self.add_at_path_inner(definition_name, &MergePath::default(), selection_set, false);
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
        trace!(
            "adding '{}' to definition_name: '{}' at path '{}'",
            selection_set,
            definition_name,
            fetch_path
        );

        let target_def = self
            .maybe_root_type_name()
            .map(|name| name.to_string())
            .unwrap_or(definition_name.to_string());
        let mut current = self.selections_for_definition_mut(&target_def);

        if let Some(selection_at_path) = find_selection_set_by_path_mut(&mut current, &fetch_path) {
            merge_selection_set(selection_at_path, &selection_set, as_first);
        } else {
            // TODO: Replace with error handling
            panic!(
                "[{}] Path '{}' cannot be found in selection set: '{}'",
                target_def, fetch_path, current
            );
        }
    }
}

impl PartialEq<TypeAwareSelection> for FetchStepSelections {
    fn eq(&self, other: &TypeAwareSelection) -> bool {
        if let Some(selections) = self.selections_for_definition(&other.type_name) {
            selections.eq(&other.selection_set)
        } else {
            false
        }
    }
}
