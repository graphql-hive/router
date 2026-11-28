use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    marker::PhantomData,
    rc::Rc,
};

use crate::query_planner::{
    ast::{
        merge_path::{Condition, MergePath, Segment},
        selection_item::SelectionItem,
        selection_set::{
            find_selection_set_by_path_mut, find_selection_sets_by_path_mut, merge_mutation_root,
            merge_selection_set, merge_selection_set_owned, response_keys_conflict,
            selection_items_are_subset_of, FieldSelection, InlineFragmentSelection,
            ResponseKeyConflict, SelectionSet,
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
    #[error(transparent)]
    ResponseKeyConflict(#[from] ResponseKeyConflict),
}

#[derive(Debug, Clone)]
pub struct FetchStepSelections<State> {
    selections: BTreeMap<String, SelectionSet>,
    _state: PhantomData<State>,
}

/// Where the objects of `type_name` are, when a step's selections move to `fetch_path` of
/// another step. With one type that's `fetch_path`. With several, like `search.@|[Book|Movie]`
/// or `search.@`, each type's fields go under its own type condition there: `search.@|[Movie]`.
fn path_of_type(several: bool, fetch_path: &MergePath, type_name: &str) -> MergePath {
    if !several || fetch_path.is_empty() {
        return fetch_path.clone();
    }
    match fetch_path.last() {
        Some(Segment::TypeCondition(..)) => {
            fetch_path.with_only_type_at(fetch_path.len() - 1, type_name)
        }
        _ => fetch_path.push(Segment::TypeCondition(
            Rc::new(BTreeSet::from([type_name.to_string()])),
            None,
        )),
    }
}

impl Display for FetchStepSelections<SingleTypeFetchStep> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_selection_set: SelectionSet = self.into();

        write!(f, "{}", as_selection_set)
    }
}

impl FetchStepSelections<MultiTypeFetchStep> {
    /// Meant for input and output of entity calls.
    pub fn to_non_root_selection_set(&self) -> SelectionSet {
        self.wrap_in_type_fragments()
    }

    pub fn to_root_selection_set(&self, root_type_name: &str) -> SelectionSet {
        if self.selections.len() == 1 {
            let (type_name, selections) = self.selections.iter().next().unwrap();

            if type_name == root_type_name {
                return selections.clone();
            }
        }

        self.wrap_in_type_fragments()
    }

    fn wrap_in_type_fragments(&self) -> SelectionSet {
        SelectionSet {
            items: self
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
                        include_if: condition.to_include_if(),
                        skip_if: condition.to_skip_if(),
                        selections: selections_for_wrapper,
                    })
                })
                .collect(),
        }
    }
}

fn inline_fragment_condition(fragment: &InlineFragmentSelection) -> Option<Condition> {
    fragment.into()
}

// Returns a condition only when every top-level selection item is an inline fragment
// with the same type condition and all those fragments use the exact same condition (include/skip).
// A mixed selection like `isCheap` and `name @include($showName)` must stay scoped at selection level,
// otherwise `isCheap` would incorrectly disappear when `$showName` is false.
fn shared_top_level_fragment_condition(
    type_name: &str,
    selection_set: &SelectionSet,
) -> Option<Condition> {
    let mut condition = None;

    for item in selection_set.items.iter() {
        let SelectionItem::InlineFragment(fragment) = item else {
            return None;
        };

        if fragment.type_condition != type_name {
            return None;
        }

        let fragment_condition = inline_fragment_condition(fragment)?;

        match &condition {
            Some(condition) => {
                if condition != &fragment_condition {
                    return None;
                }
            }
            None => condition = Some(fragment_condition),
        }
    }

    condition
}

// Clears only the top-level wrapper condition after it has been lifted to the
// fetch step. Inner field/fragment conditions are preserved because those still
// describe selection-level behavior, not whole-fetch behavior.
fn clear_top_level_fragment_conditions(type_name: &str, selection_set: &mut SelectionSet) {
    for item in selection_set.items.iter_mut() {
        if let SelectionItem::InlineFragment(fragment) = item {
            if fragment.type_condition == type_name {
                fragment.include_if = None;
                fragment.skip_if = None;
            }
        }
    }
}

/// Attempts to lift a common condition from the top-level inline fragments for
/// `type_name` into the wrapper `... on Type` fragment we build in `wrap_in_type_fragments`.
///
/// Lifting is valid when every top-level item is an inline fragment on the same
/// type and they all share the same condition. That condition may be
/// `@include`, `@skip`, or both directives together
/// (`Condition::SkipAndInclude`).
fn try_lift_condition(
    type_name: &str,
    selections: &SelectionSet,
) -> Option<(Condition, SelectionSet)> {
    let first_item = selections.items.first()?;
    let SelectionItem::InlineFragment(first_fragment) = first_item else {
        return None;
    };

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

        // `... on Cat @include(if: $x)` under `Animal` is a type condition as well, lifting
        // only the `@include` would lose it.
        inline_fragment.type_condition == type_name
            && inline_fragment_condition(inline_fragment).as_ref() == Some(&condition)
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

    /// Adds `selection_set` at `fetch_path` of `definition_name`, or of the only type there is.
    pub fn add_at_path(
        &mut self,
        definition_name: Option<&str>,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
    ) -> Result<(), FetchStepSelectionsError> {
        self.add_at_path_of(definition_name, fetch_path, selection_set, false)
    }

    pub fn add_selection_typename(
        &mut self,
        definition_name: Option<&str>,
        fetch_path: &MergePath,
    ) -> Result<(), FetchStepSelectionsError> {
        self.add_at_path_of(
            definition_name,
            fetch_path,
            SelectionSet {
                items: vec![SelectionItem::Field(FieldSelection::new_typename())],
            },
            true,
        )
    }

    fn add_at_path_of(
        &mut self,
        definition_name: Option<&str>,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
        as_first: bool,
    ) -> Result<(), FetchStepSelectionsError> {
        let current = match definition_name {
            Some(definition_name) => self.selections.get_mut(definition_name),
            None => self.selections.values_mut().next(),
        }
        .ok_or_else(|| {
            FetchStepSelectionsError::UnexpectedMissingDefinition(
                definition_name.unwrap_or_default().to_string(),
            )
        })?;
        add_at_each_path(
            current,
            fetch_path,
            selection_set,
            as_first,
            definition_name.unwrap_or_default(),
        )
    }
}

/// Merges `selection_set` into `current` at `fetch_path`, and when a type condition on it has
/// several types, under each of them: a step that took the objects of several branches, like
/// `product.|[Book|Movie].reviews.@`, merged into one that has them apart.
fn add_at_each_path(
    current: &mut SelectionSet,
    fetch_path: &MergePath,
    selection_set: SelectionSet,
    as_first: bool,
    definition_name: &str,
) -> Result<(), FetchStepSelectionsError> {
    let missing = || {
        FetchStepSelectionsError::MissingPathInSelection(
            fetch_path.to_string(),
            definition_name.to_string(),
        )
    };
    let several_types = fetch_path.inner.iter().any(
        |segment| matches!(segment, Segment::TypeCondition(type_names, _) if type_names.len() > 1),
    );
    if !several_types {
        let at_path = find_selection_set_by_path_mut(current, fetch_path).ok_or_else(missing)?;
        merge_selection_set_owned(at_path, selection_set, as_first)?;
        return Ok(());
    }
    let mut found = Vec::new();
    if !find_selection_sets_by_path_mut(current, &fetch_path.inner, &mut found) {
        return Err(missing());
    }
    // the last one takes `selection_set` itself, the others a copy
    let last = found.pop();
    for at_path in found {
        merge_selection_set_owned(at_path, selection_set.clone(), as_first)?;
    }
    if let Some(at_path) = last {
        merge_selection_set_owned(at_path, selection_set, as_first)?;
    }
    Ok(())
}

impl<State> FetchStepSelections<State> {
    /// The first type name, which on a single-type step is its only one.
    pub fn first_definition_name(&self) -> Option<&str> {
        self.selections.keys().next().map(String::as_str)
    }

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

    pub fn iter_selections_mut(&mut self) -> impl Iterator<Item = (&str, &mut SelectionSet)> {
        self.selections
            .iter_mut()
            .map(|(name, selection_set)| (name.as_str(), selection_set))
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
}

impl FetchStepSelections<MultiTypeFetchStep> {
    fn wrap_definition_selection_with_condition(
        def_name: &str,
        selection_set: &mut SelectionSet,
        condition: &Condition,
    ) {
        // A type slot can be declared before it gets any selections (multi-type batching does
        // that right before a merge). Nothing to guard there, and `... on T @include { }` is
        // invalid GraphQL that ends up in the subgraph request.
        if selection_set.items.is_empty() {
            return;
        }
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
            Condition::SkipAndInclude { skip, include } => {
                selection_set.items =
                    vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                        type_condition: def_name.to_string(),
                        selections: prev,
                        skip_if: Some(skip.clone()),
                        include_if: Some(include.clone()),
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

    /// Whether a type both select would get a response key with two values, see
    /// `response_keys_conflict`.
    pub fn response_keys_conflict_with(&self, other: &Self) -> bool {
        self.iter_selections().any(|(definition_name, selections)| {
            other
                .selections
                .get(definition_name)
                .is_some_and(|other_selections| {
                    response_keys_conflict(selections, other_selections)
                })
        })
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

    pub fn replace_definitions_with_abstract(
        &mut self,
        abstract_type: &str,
        selection_set: SelectionSet,
    ) {
        self.selections.clear();
        self.selections
            .insert(abstract_type.to_string(), selection_set);
    }

    /// Moves `other`'s selections to `fetch_path` here: into `definition_name` when given, into
    /// the only type here when there's one, else each into its own type. With several types in
    /// `other`, each type's go under its own type condition, see `path_of_type`.
    pub fn migrate(
        &mut self,
        other: Self,
        fetch_path: &MergePath,
        definition_name: Option<&str>,
    ) -> Result<(), FetchStepSelectionsError> {
        let into = definition_name
            .or_else(|| self.try_as_single())
            .map(str::to_string);
        let several = other.is_fetching_multiple_types();
        for (other_definition_name, selection_set) in other.selections {
            let path = path_of_type(several, fetch_path, &other_definition_name);
            let definition_name = into.as_deref().unwrap_or(&other_definition_name);
            let current = self
                .selections_for_definition_mut(definition_name)
                .ok_or_else(|| {
                    FetchStepSelectionsError::UnexpectedMissingDefinition(
                        definition_name.to_string(),
                    )
                })?;
            add_at_each_path(current, &path, selection_set, false, definition_name)?;
        }
        Ok(())
    }

    /// Adds `other` after ours at a mutation's root, see `merge_mutation_root`.
    pub fn migrate_mutation_root(&mut self, other: Self) -> Result<(), FetchStepSelectionsError> {
        for (definition_name, selection_set) in other.selections {
            let current = self
                .selections_for_definition_mut(&definition_name)
                .ok_or_else(|| {
                    FetchStepSelectionsError::UnexpectedMissingDefinition(definition_name.clone())
                })?;
            merge_mutation_root(current, selection_set)?;
        }
        Ok(())
    }

    /// Takes the selections out, leaving none.
    pub fn take(&mut self) -> Self {
        Self {
            selections: std::mem::take(&mut self.selections),
            _state: Default::default(),
        }
    }

    pub fn remove_definition(&mut self, definition_name: &str) {
        self.selections.remove(definition_name);
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

    // Tries to lift a shared output condition into the fetch step.
    // A step-level condition means the whole fetch can be skipped.
    // `Some` only when every type is guarded by the same top-level condition.
    // The lifted wrapper directives are cleared from the output
    // to avoid duplicating the same condition at both fetch and selection level.
    pub fn take_shared_top_level_fragment_condition(&mut self) -> Option<Condition> {
        if self.is_empty() {
            return None;
        }

        let mut common_condition = None;

        for (type_name, selection_set) in self.iter() {
            let condition = shared_top_level_fragment_condition(type_name, selection_set)?;

            match &common_condition {
                Some(common_condition) => {
                    if common_condition != &condition {
                        return None;
                    }
                }
                None => common_condition = Some(condition),
            }
        }

        let condition = common_condition?;
        for (type_name, selection_set) in self.selections.iter_mut() {
            clear_top_level_fragment_conditions(type_name, selection_set);
        }

        Some(condition)
    }
}

impl FetchStepSelections<SingleTypeFetchStep> {
    /// Adds to the only type of a step that has one.
    pub fn add(&mut self, selection_set: &SelectionSet) -> Result<(), FetchStepSelectionsError> {
        debug_assert!(self.selections.len() == 1, "the step takes several types");
        let current = self
            .selections
            .values_mut()
            .next()
            .expect("SingleTypeFetchStep should have a selection");
        merge_selection_set(current, selection_set, false)?;

        Ok(())
    }

    pub fn add_to(
        &mut self,
        definition_name: &str,
        selection_set: &SelectionSet,
    ) -> Result<(), FetchStepSelectionsError> {
        let current = self
            .selections_for_definition_mut(definition_name)
            .ok_or_else(|| {
                FetchStepSelectionsError::UnexpectedMissingDefinition(definition_name.to_string())
            })?;
        merge_selection_set(current, selection_set, false)?;

        Ok(())
    }

    /// Makes room for another type, for an entity call that takes several.
    pub fn add_definition(&mut self, definition_name: &str) {
        self.selections
            .entry(definition_name.to_string())
            .or_default();
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

    use crate::query_planner::ast::selection_item::SelectionItem;
    use crate::query_planner::ast::selection_set::SelectionSet;
    use crate::query_planner::utils::parsing::parse_operation;
    use crate::query_planner::utils::pretty_display::PrettyDisplay;

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
            format!("{}", PrettySelectionSet(fetch_selections.to_non_root_selection_set())),
            @r#"
              ... on Book @skip(if: $title) {
                sku
              }
            "#
        );
    }

    #[test]
    fn lifts_shared_condition_from_multiple_fragments() {
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
          format!("{}", PrettySelectionSet(fetch_selections.to_non_root_selection_set())),
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
          format!("{}", PrettySelectionSet(fetch_selections.to_non_root_selection_set())),
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
          format!("{}", PrettySelectionSet(fetch_selections.to_non_root_selection_set())),
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
