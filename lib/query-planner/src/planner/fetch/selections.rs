use std::{
    collections::{BTreeSet, HashMap},
    fmt::Display,
    marker::PhantomData,
};

use crate::{
    ast::{
        merge_path::{Condition, MergePath},
        safe_merge::{AliasesRecords, SafeSelectionSetMerger},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
        type_aware_selection::{find_selection_set_by_path_mut, merge_selection_set},
    },
    planner::fetch::state::{MultiTypeFetchStep, SingleTypeFetchStep},
};

#[derive(Debug, thiserror::Error)]
pub enum FetchStepSelectionsError {
    #[error("Unexpected missing definition: {0}")]
    UnexpectedMissingDefinition(String),
    #[error("Path '{0}' cannot be found in selection set of type {1}")]
    MissingPathInSelection(String, String),
}

#[derive(Debug, Clone)]
pub struct FetchStepSelections<State> {
    selections: HashMap<String, SelectionSet>,
    _state: PhantomData<State>,
}

impl Display for FetchStepSelections<SingleTypeFetchStep> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_selection_set: SelectionSet = self.into();

        write!(f, "{}", as_selection_set)
    }
}

impl From<&FetchStepSelections<MultiTypeFetchStep>> for SelectionSet {
    fn from(value: &FetchStepSelections<MultiTypeFetchStep>) -> Self {
        if value.selections.len() == 1 {
            let (type_name, selections) = value.selections.iter().next().unwrap();

            if type_name == "Query" || type_name == "Mutation" || type_name == "Subscription" {
                return selections.clone();
            }
        }

        SelectionSet {
            items: value
                .selections
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

impl From<&FetchStepSelections<SingleTypeFetchStep>> for SelectionSet {
    fn from(value: &FetchStepSelections<SingleTypeFetchStep>) -> Self {
        let (_type_name, selections) = value.selections.iter().next().unwrap();

        selections.clone()
    }
}

impl FetchStepSelections<SingleTypeFetchStep> {
    pub fn into_multi_type(self) -> FetchStepSelections<MultiTypeFetchStep> {
        FetchStepSelections {
            _state: Default::default(),
            selections: self.selections,
        }
    }

    pub fn definition_name(&self) -> &str {
        self.selections
            .keys()
            .next()
            .expect("SingleTypeFetchStep should have exactly one selection")
    }

    pub fn selection_set(&self) -> &SelectionSet {
        self.selections
            .iter()
            .next()
            .expect("SingleTypeFetchStep should have exactly one selection")
            .1
    }

    pub fn add_at_path(
        &mut self,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
    ) -> Result<(), FetchStepSelectionsError> {
        let def_name = self.definition_name().to_string();

        self.add_at_path_inner(&def_name, fetch_path, selection_set, false)
    }

    pub fn add_selection_typename(
        &mut self,
        fetch_path: &MergePath,
    ) -> Result<(), FetchStepSelectionsError> {
        let def_name = self.definition_name().to_string();

        self.add_at_path_inner(
            &def_name,
            fetch_path,
            SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection::new_typename())],
            },
            true,
        )
    }
}

impl<State> FetchStepSelections<State> {
    pub fn is_selecting_definition(&self, definition_name: &str) -> bool {
        self.selections.contains_key(definition_name)
    }

    pub fn is_empty(&self) -> bool {
        self.selections.is_empty()
            || self
                .selections
                .values()
                .all(|selection_set| selection_set.is_empty())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &SelectionSet)> {
        self.selections.iter()
    }

    pub fn variable_usages(&self) -> BTreeSet<String> {
        let mut usages = BTreeSet::new();

        for selection_set in self.selections.values() {
            usages.extend(selection_set.variable_usages());
        }

        usages
    }

    pub fn selections_for_definition_mut(
        &mut self,
        definition_name: &str,
    ) -> Option<&mut SelectionSet> {
        self.selections.get_mut(definition_name)
    }

    fn add_at_path_inner(
        &mut self,
        definition_name: &str,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
        as_first: bool,
    ) -> Result<(), FetchStepSelectionsError> {
        let current = self
            .selections_for_definition_mut(definition_name)
            .ok_or_else(|| {
                FetchStepSelectionsError::UnexpectedMissingDefinition(definition_name.to_string())
            })?;

        let selection_set_at_path = find_selection_set_by_path_mut(current, fetch_path)
            .ok_or_else(|| {
                FetchStepSelectionsError::MissingPathInSelection(
                    fetch_path.to_string(),
                    definition_name.to_string(),
                )
            })?;

        merge_selection_set(selection_set_at_path, &selection_set, as_first);

        Ok(())
    }
}

impl FetchStepSelections<MultiTypeFetchStep> {
    pub fn try_as_single(&self) -> Option<&str> {
        if self.selections.len() == 1 {
            self.selections.keys().next().map(|key| key.as_str())
        } else {
            None
        }
    }

    pub fn iter_selections(&self) -> impl Iterator<Item = (&String, &SelectionSet)> {
        self.selections.iter()
    }

    pub fn migrate_from_another(
        &mut self,
        other: &Self,
        fetch_path: &MergePath,
    ) -> Result<(), FetchStepSelectionsError> {
        let maybe_merge_into = self.try_as_single().map(|str| str.to_string());

        for (definition_name, selection_set) in other.iter_selections() {
            let target_type = maybe_merge_into.as_ref().unwrap_or(definition_name);
            self.add_at_path_inner(target_type, fetch_path, selection_set.clone(), false)?;
        }

        Ok(())
    }

    pub fn add(
        &mut self,
        definition_name: &str,
        selection_set: &SelectionSet,
    ) -> Result<(), FetchStepSelectionsError> {
        let selections = self
            .selections_for_definition_mut(definition_name)
            .ok_or_else(|| {
                FetchStepSelectionsError::UnexpectedMissingDefinition(definition_name.to_string())
            })?;

        merge_selection_set(selections, selection_set, false);

        Ok(())
    }

    pub fn safe_migrate_from_another(
        &mut self,
        other: &Self,
        fetch_path: &MergePath,
        (self_used_for_requires, other_used_for_requires): (bool, bool),
    ) -> Result<Vec<(String, AliasesRecords)>, FetchStepSelectionsError> {
        let mut aliases_made: Vec<(String, AliasesRecords)> = Vec::new();
        let maybe_merge_into = self.try_as_single().map(|str| str.to_string());

        for (definition_name, selection_set) in other.iter_selections() {
            let target_type = maybe_merge_into.as_ref().unwrap_or(definition_name);
            let current = self
                .selections_for_definition_mut(target_type)
                .ok_or_else(|| {
                    FetchStepSelectionsError::UnexpectedMissingDefinition(target_type.to_string())
                })?;

            let selection_at_path = find_selection_set_by_path_mut(current, fetch_path)
                .ok_or_else(|| {
                    FetchStepSelectionsError::MissingPathInSelection(
                        fetch_path.to_string(),
                        target_type.to_string(),
                    )
                })?;

            let mut merger = SafeSelectionSetMerger::default();
            let current_aliases_made = merger.merge_selection_set(
                selection_at_path,
                selection_set,
                (self_used_for_requires, other_used_for_requires),
                false,
            );

            if !current_aliases_made.is_empty() {
                aliases_made.push((target_type.to_string(), current_aliases_made));
            }
        }

        Ok(aliases_made)
    }

    pub fn wrap_with_condition(&mut self, condition: Condition) {
        for (def_name, selection_set) in self.selections.iter_mut() {
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

impl FetchStepSelections<SingleTypeFetchStep> {
    pub fn new(definition_name: &str) -> Self {
        let mut map = HashMap::new();
        map.insert(definition_name.to_string(), SelectionSet::default());

        Self {
            _state: Default::default(),
            selections: map,
        }
    }

    pub fn new_empty() -> Self {
        Self {
            _state: Default::default(),
            selections: Default::default(),
        }
    }
}
