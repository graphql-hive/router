use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    marker::PhantomData,
};

use crate::{
    ast::{
        merge_path::{Condition, MergePath},
        safe_merge::{AliasesRecords, SafeSelectionSetMerger},
        selection_item::SelectionItem,
        selection_set::{
            find_selection_set_by_path_mut, merge_selection_set, selection_items_are_subset_of,
            FieldSelection, InlineFragmentSelection, SelectionSet,
        },
    },
    planner::fetch::state::{MultiTypeFetchStep, SingleTypeFetchStep},
};

#[derive(Debug, thiserror::Error, Clone)]
pub enum FetchStepSelectionsError {
    #[error("Unexpected missing definition: {0}")]
    UnexpectedMissingDefinition(String),
    #[error("Path '{0}' cannot be found in selection set of type {1}")]
    MissingPathInSelection(String, String),
}

#[derive(Debug, Clone)]
pub struct FetchStepSelections<State> {
    selections: BTreeMap<String, SelectionSet>,
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
                .map(|(type_name, selections)| {
                    // Build one `... on Type` wrapper item if needed
                    let Some((condition, selections_for_wrapper)) =
                        try_lift_condition(type_name, selections)
                    else {
                        return SelectionItem::InlineFragment(InlineFragmentSelection {
                            type_condition: type_name.to_string(),
                            include_if: None,
                            skip_if: None,
                            selections: selections.clone(),
                        });
                    };

                    SelectionItem::InlineFragment(InlineFragmentSelection {
                        type_condition: type_name.to_string(),
                        include_if: match &condition {
                            Condition::Include(var_name) => Some(var_name.clone()),
                            Condition::Skip(_) => None,
                        },
                        skip_if: match &condition {
                            Condition::Skip(var_name) => Some(var_name.clone()),
                            Condition::Include(_) => None,
                        },
                        selections: selections_for_wrapper,
                    })
                })
                .collect(),
        }
    }
}

fn inline_fragment_condition(fragment: &InlineFragmentSelection) -> Option<Condition> {
    // Return a condition:
    match (fragment.include_if.as_ref(), fragment.skip_if.as_ref()) {
        // Either @include
        (Some(var_name), None) => Some(Condition::Include(var_name.clone())),
        // or @skip
        (None, Some(var_name)) => Some(Condition::Skip(var_name.clone())),
        // not when both are available
        _ => None,
    }
}

fn try_lift_condition(
    type_name: &str,
    selections: &SelectionSet,
) -> Option<(Condition, SelectionSet)> {
    let first_item = selections.items.first()?;
    let SelectionItem::InlineFragment(first_fragment) = first_item else {
        return None;
    };
    debug_assert_eq!(first_fragment.type_condition, type_name);

    // Use the first fragment as baseline condition.
    // Valid means exactly one conditional directive:
    // - @include(if: $x), or
    // - @skip(if: $x)
    // If it has neither or both, we do not lift.
    let condition = inline_fragment_condition(first_fragment)?;

    // Every top-level item must be an inline fragment with the same condition.
    // If this fails, lifting would change semantics.
    // Here's what i mean:
    //   ... on User @include(if: $show) { id }
    //   ... on User @skip(if: $show) { name }
    // should not be lifted.
    let all_match = selections.items.iter().all(|item| {
        let SelectionItem::InlineFragment(inline_fragment) = item else {
            return false;
        };

        debug_assert_eq!(inline_fragment.type_condition, type_name);
        inline_fragment_condition(inline_fragment).as_ref() == Some(&condition)
    });

    if !all_match {
        return None;
    }

    // With one fragment, use its selections directly.
    // This avoids creating an extra nested `... on Type` fragment.
    //   ... on User @include(if: $show) { id }
    // should not become:
    //   ... on User @include(if: $show) { ... on User { id } }
    if selections.items.len() == 1 {
        return Some((condition, first_fragment.selections.clone()));
    }

    // We lifted the condition to the outer fragment.
    // Remove identical inner conditions to avoid duplicated nesting.
    // Before
    //  ... on User @include(if: $show) { id }
    //  ... on User @include(if: $show) { name }
    // After
    //  ... on User @include(if: $show) {
    //    ... on User { id }
    //    ... on User { name }
    //  }
    let mut lifted_selections = selections.clone();
    for item in lifted_selections.items.iter_mut() {
        if let SelectionItem::InlineFragment(inline_fragment) = item {
            inline_fragment.include_if = None;
            inline_fragment.skip_if = None;
        }
    }

    Some((condition, lifted_selections))
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

    pub fn selection_set_mut(&mut self) -> &mut SelectionSet {
        self.selections
            .iter_mut()
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
    pub fn is_fetching_multiple_types(&self) -> bool {
        self.selections.len() > 1
    }

    pub fn contains(&self, definition_name: &str, selection_set: &SelectionSet) -> bool {
        if let Some(self_selections) = self.selections.get(definition_name) {
            return selection_items_are_subset_of(&self_selections.items, &selection_set.items);
        }

        false
    }

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

    pub fn selections_for_definition(&self, definition_name: &str) -> Option<&SelectionSet> {
        self.selections.get(definition_name)
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
    fn wrap_definition_selection_with_condition(
        def_name: &str,
        selection_set: &mut SelectionSet,
        condition: &Condition,
    ) {
        let prev = selection_set.clone();
        match condition {
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

    pub fn selecting_same_types(&self, other: &Self) -> bool {
        if self.selections.len() != other.selections.len() {
            return false;
        }

        for key in self.selections.keys() {
            if !other.selections.contains_key(key) {
                return false;
            }
        }

        true
    }

    pub fn iter_matching_types<'a, 'b, R>(
        input: &'a FetchStepSelections<MultiTypeFetchStep>,
        other: &'b FetchStepSelections<MultiTypeFetchStep>,
        mut callback: impl FnMut(&str, &SelectionSet, &SelectionSet) -> R,
    ) -> Vec<(&'a str, R)> {
        let mut result: Vec<(&'a str, R)> = Vec::new();

        for (definition_name, input_selections) in input.iter_selections() {
            if let Some(other_selections) = other.selections.get(definition_name) {
                let r = callback(definition_name, input_selections, other_selections);
                result.push((definition_name, r));
            }
        }

        result
    }

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

    /// Creates a slot in the internal HashMap and will allow to add selections for the given definition name.
    /// Without that, trying to add selections using any function will either fail or result in trying to force-add to a root type.
    /// Calling this method is crucial if you wish to create multi-type steps.
    pub fn declare_known_type(&mut self, def_name: &str) {
        self.selections.entry(def_name.to_string()).or_default();
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
            Self::wrap_definition_selection_with_condition(def_name, selection_set, &condition);
        }
    }

    pub fn wrap_with_condition_for_types(
        &mut self,
        condition: Condition,
        type_names: &BTreeSet<&str>,
    ) {
        for (def_name, selection_set) in self.selections.iter_mut() {
            if type_names.contains(def_name.as_str()) {
                Self::wrap_definition_selection_with_condition(def_name, selection_set, &condition);
            }
        }
    }
}

impl FetchStepSelections<SingleTypeFetchStep> {
    pub fn add(&mut self, selection_set: &SelectionSet) -> Result<(), FetchStepSelectionsError> {
        merge_selection_set(self.selection_set_mut(), selection_set, false);

        Ok(())
    }

    pub fn new(definition_name: &str) -> Self {
        let mut map = BTreeMap::new();
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

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, marker::PhantomData};

    use graphql_tools::parser::query::{Definition, OperationDefinition};

    use crate::ast::selection_item::SelectionItem;
    use crate::ast::selection_set::SelectionSet;
    use crate::utils::parsing::parse_operation;
    use crate::utils::pretty_display::PrettyDisplay;

    use super::{FetchStepSelections, MultiTypeFetchStep};

    fn parse_selection_set(input: &str) -> SelectionSet {
        let op = parse_operation(input);

        match op.definitions.first() {
            Some(Definition::Operation(OperationDefinition::SelectionSet(s))) => s.clone().into(),
            _ => panic!("expected top-level selection set input"),
        }
    }

    fn multi_type_from_top_level_inline_fragments(
        query: &str,
    ) -> FetchStepSelections<MultiTypeFetchStep> {
        let parsed = parse_selection_set(query);
        let mut map = BTreeMap::<String, SelectionSet>::new();

        for item in parsed.items {
            let SelectionItem::InlineFragment(inline_fragment) = item else {
                panic!("expected only top-level inline fragments in test input");
            };

            map.entry(inline_fragment.type_condition.clone())
                .or_insert_with(|| SelectionSet { items: vec![] })
                .items
                .push(SelectionItem::InlineFragment(inline_fragment));
        }

        FetchStepSelections {
            selections: map,
            _state: PhantomData,
        }
    }

    struct PrettySelectionSet(SelectionSet);

    impl std::fmt::Display for PrettySelectionSet {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.0.pretty_fmt(f, 0)
        }
    }

    #[test]
    fn lifts_single_conditional_fragment_without_extra_nesting() {
        let fetch_selections = multi_type_from_top_level_inline_fragments(
            r#"
            {
              ... on Book @skip(if: $title) {
                sku
              }
            }
            "#,
        );
        insta::assert_snapshot!(
            format!("{}", PrettySelectionSet((&fetch_selections).into())),
            @r#"
              ... on Book @skip(if: $title) {
                sku
              }
            "#
        );
    }

    #[test]
    fn lifts_uniform_condition_from_multiple_fragments() {
        let fetch_selections = multi_type_from_top_level_inline_fragments(
            r#"
            {
              ... on Book @include(if: $x) {
                title
              }
              ... on Book @include(if: $x) {
                author
              }
            }
            "#,
        );

        insta::assert_snapshot!(
          format!("{}", PrettySelectionSet((&fetch_selections).into())),
            @r#"
              ... on Book @include(if: $x) {
                ... on Book {
                  title
                }
                ... on Book {
                  author
                }
              }
            "#
        );
    }

    #[test]
    fn does_not_lift_when_conditions_are_mixed() {
        let fetch_selections = multi_type_from_top_level_inline_fragments(
            r#"
            {
              ... on Book @include(if: $x) {
                title
              }
              ... on Book {
                sku
              }
            }
            "#,
        );

        insta::assert_snapshot!(
          format!("{}", PrettySelectionSet((&fetch_selections).into())),
            @r#"
              ... on Book {
                ... on Book @include(if: $x) {
                  title
                }
                ... on Book {
                  sku
                }
              }
            "#
        );
    }

    #[test]
    fn does_not_lift_when_top_level_fragments_have_different_types() {
        let fetch_selections = multi_type_from_top_level_inline_fragments(
            r#"
            {
              ... on Book @include(if: $x) {
                title
              }
              ... on Magazine @include(if: $x) {
                sku
              }
            }
            "#,
        );

        insta::assert_snapshot!(
          format!("{}", PrettySelectionSet((&fetch_selections).into())),
            @r#"
              ... on Book @include(if: $x) {
                title
              }
              ... on Magazine @include(if: $x) {
                sku
              }
            "#
        );
    }
}
