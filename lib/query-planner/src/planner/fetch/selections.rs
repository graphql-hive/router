use std::{
    collections::{BTreeSet, HashMap},
    fmt::Display,
};

use crate::ast::{
    merge_path::{Condition, MergePath},
    safe_merge::{AliasesRecords, SafeSelectionSetMerger},
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
    pub fn new_root(root_type_name: &str) -> Self {
        Self::Root {
            type_name: root_type_name.to_string(),
            selection_set: SelectionSet::default(),
        }
    }

    pub fn type_name(&self) -> &str {
        match self {
            Self::Root { type_name, .. } => type_name,
            Self::Entities { selections } => selections.keys().next().unwrap(),
        }
    }

    pub fn new_entities(entity_name: &str) -> Self {
        let mut map = HashMap::new();
        map.insert(entity_name.to_string(), SelectionSet::default());

        Self::Entities { selections: map }
    }

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
            Self::Entities { selections } => {
                if let Some(selection_set) = selections.get_mut(definition_name) {
                    selection_set
                } else {
                    panic!(
                        "failed to lookup entity type.  requested: {}",
                        definition_name
                    )
                }
            }
        }
    }

    pub fn is_selecting_definition(&self, definition_name: &str) -> bool {
        match self {
            Self::Root { type_name, .. } => type_name == definition_name,
            Self::Entities { selections } => selections.contains_key(definition_name),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Root { selection_set, .. } => selection_set.is_empty(),
            Self::Entities { selections } => {
                selections.is_empty()
                    || selections
                        .values()
                        .all(|selection_set| selection_set.is_empty())
            }
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

    pub fn migrate_from_another(&mut self, other: &Self, fetch_path: &MergePath) {
        for (definition_name, selection_set) in other.selections() {
            println!(
                "migrate_from_another, fetch_path: {}, definition_name={}",
                fetch_path, definition_name
            );

            self.add_at_path_inner(fetch_path, selection_set.clone(), false);
        }
    }

    pub fn selections(&self) -> Box<dyn Iterator<Item = (&String, &SelectionSet)> + '_> {
        match self {
            Self::Root {
                selection_set,
                type_name,
            } => Box::new(std::iter::once((type_name, selection_set))),
            Self::Entities { selections } => Box::new(selections.iter()),
        }
    }

    pub fn add_at_path(&mut self, fetch_path: &MergePath, selection_set: SelectionSet) {
        self.add_at_path_inner(fetch_path, selection_set, false);
    }

    pub fn add_selection_typename(&mut self, fetch_path: &MergePath) {
        self.add_at_path_inner(
            fetch_path,
            SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection::new_typename())],
            },
            true,
        );
    }

    fn get_definition_to_modify<'a>(
        &'a self,
        fetch_path: &'a MergePath,
        fallback_definition_name: &'a str,
    ) -> &'a str {
        if !fetch_path.is_empty() {
            if let Self::Root { type_name, .. } = self {
                type_name.as_str()
            } else {
                &fetch_path.entrypoint_definition_name
            }
        } else {
            fallback_definition_name
        }
    }

    pub fn add(&mut self, definition_name: &str, selection_set: &SelectionSet) {
        let selections = self.selections_for_definition(definition_name);
        merge_selection_set(selections, selection_set, false);
    }

    pub fn safe_add_from_another_at_path(
        &mut self,
        other: &Self,
        fetch_path: &MergePath,
        (self_used_for_requires, other_used_for_requires): (bool, bool),
    ) -> Vec<(String, AliasesRecords)> {
        let mut aliases_made: Vec<(String, AliasesRecords)> = Vec::new();

        for (definition_name, selection_set) in other.selections() {
            let target_type_name = self
                .get_definition_to_modify(fetch_path, definition_name)
                .to_string();
            let current = self.selections_for_definition(&target_type_name);

            if let Some(selection_at_path) = find_selection_set_by_path_mut(current, fetch_path) {
                let mut merger = SafeSelectionSetMerger::default();
                let current_aliases_made = merger.merge_selection_set(
                    selection_at_path,
                    selection_set,
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

    pub fn wrap_with_condition(&mut self, condition: Condition) {
        match self {
            Self::Root {
                type_name,
                selection_set,
            } => {
                let prev = selection_set.clone();

                match &condition {
                    Condition::Include(var_name) => {
                        selection_set.items =
                            vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                                type_condition: type_name.to_string(),
                                selections: prev,
                                skip_if: None,
                                include_if: Some(var_name.clone()),
                            })];
                    }
                    Condition::Skip(var_name) => {
                        selection_set.items =
                            vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                                type_condition: type_name.to_string(),
                                selections: prev,
                                skip_if: Some(var_name.clone()),
                                include_if: None,
                            })];
                    }
                }
            }
            Self::Entities { selections } => {
                for (def_name, selection_set) in selections {
                    let prev = selection_set.clone();
                    match &condition {
                        Condition::Include(var_name) => {
                            selection_set.items =
                                vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                                    type_condition: def_name.to_string(),
                                    selections: prev,
                                    skip_if: None,
                                    include_if: Some(var_name.clone()),
                                })];
                        }
                        Condition::Skip(var_name) => {
                            selection_set.items =
                                vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                                    type_condition: def_name.to_string(),
                                    selections: prev,
                                    skip_if: Some(var_name.clone()),
                                    include_if: None,
                                })];
                        }
                    }
                }
            }
        }
    }

    fn add_at_path_inner(
        &mut self,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
        as_first: bool,
    ) {
        let target_def = self
            .get_definition_to_modify(fetch_path, &fetch_path.entrypoint_definition_name)
            .to_string();
        let current = self.selections_for_definition(&target_def);

        if let Some(selection_at_path) = find_selection_set_by_path_mut(current, fetch_path) {
            merge_selection_set(selection_at_path, &selection_set, as_first);
        } else {
            panic!(
                "[{}] Path '{}' cannot be found in selection set: '{}'",
                target_def, fetch_path, current
            );
        }
    }
}
