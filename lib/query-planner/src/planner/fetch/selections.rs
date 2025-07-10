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

#[derive(Debug, Clone)]
pub enum FetchStepSelections {
    Root {
        type_name: String,
        selection_set: SelectionSet,
    },
    Entities {
        selections: HashMap<String, SelectionSet>,
    },
}

impl Display for FetchStepSelections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_selection_set: SelectionSet = self.into();

        write!(f, "{}", as_selection_set)
    }
}

impl From<&FetchStepSelections> for SelectionSet {
    fn from(value: &FetchStepSelections) -> Self {
        match value {
            FetchStepSelections::Root { selection_set, .. } => selection_set.clone(),
            FetchStepSelections::Entities { selections } => SelectionSet {
                items: selections
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
            },
        }
    }
}

impl FetchStepSelections {
    pub fn selections_for_definition(&mut self, definition_name: &str) -> &mut SelectionSet {
        match self {
            Self::Root {
                type_name,
                selection_set,
            } => {
                if type_name == definition_name {
                    selection_set
                } else {
                    panic!(
                        "failed to lookup root type. current: {}, requested: {}",
                        type_name, definition_name
                    )
                }
            }
            Self::Entities { selections } => selections
                .entry(definition_name.to_string())
                .or_insert_with(SelectionSet::default),
        }
    }

    pub fn is_selecting_definition(&self, definition_name: &str) -> bool {
        match self {
            Self::Root { type_name, .. } => type_name == definition_name,
            Self::Entities { selections } => selections.contains_key(definition_name),
        }
    }

    pub fn variable_usages(&self) -> BTreeSet<String> {
        let mut usages = BTreeSet::new();

        match self {
            Self::Root { selection_set, .. } => {
                usages.extend(selection_set.variable_usages());
            }
            Self::Entities { selections } => {
                for selection_set in selections.values() {
                    usages.extend(selection_set.variable_usages());
                }
            }
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

    fn get_definition_to_modify<'a>(
        &'a self,
        target_def_name: &'a str,
        fetch_step: &MergePath,
    ) -> &'a str {
        if !fetch_step.is_empty() {
            if let Self::Root { type_name, .. } = self {
                type_name.as_str()
            } else {
                target_def_name
            }
        } else {
            target_def_name
        }
    }

    fn add_at_path_inner(
        &mut self,
        definition_name: &str,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
        as_first: bool,
    ) {
        let target_def = self
            .get_definition_to_modify(definition_name, fetch_path)
            .to_string();
        let mut current = self.selections_for_definition(&target_def);

        if let Some(selection_at_path) = find_selection_set_by_path_mut(&mut current, &fetch_path) {
            merge_selection_set(selection_at_path, &selection_set, as_first);
        } else {
            panic!(
                "[{}] Path '{}' cannot be found in selection set: '{}'",
                target_def, fetch_path, current
            );
        }
    }
}
