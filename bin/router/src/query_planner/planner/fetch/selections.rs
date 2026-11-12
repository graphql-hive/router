use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    marker::PhantomData,
};

use crate::query_planner::{
    ast::{
        flat::{
            arena::{Arena, ListId},
            Kind,
        },
        merge_path::{Condition, MergePath},
        selection_item::SelectionItem,
        selection_set::{
            FieldSelection, InlineFragmentSelection, ResponseKeyConflict, SelectionSet,
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

/// The selections of a fetch step's input or output, a list in the fetch graph's `Arena` per type.
/// Every list belongs to this step: a copy for another step goes through `copy`.
#[derive(Debug)]
pub struct FetchStepSelections<State> {
    selections: BTreeMap<String, ListId>,
    _state: PhantomData<State>,
}

/// Prints the selections like the planner's tree does.
pub struct DisplaySelections<'a, State> {
    selections: &'a FetchStepSelections<State>,
    arena: &'a Arena,
}

impl Display for DisplaySelections<'_, SingleTypeFetchStep> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.arena.export(self.selections.selection_set()))
    }
}

impl<State> FetchStepSelections<State> {
    pub fn display<'a>(&'a self, arena: &'a Arena) -> DisplaySelections<'a, State> {
        DisplaySelections {
            selections: self,
            arena,
        }
    }

    /// A copy whose lists are its own.
    pub fn copy(&self, arena: &mut Arena) -> Self {
        Self {
            selections: self
                .selections
                .iter()
                .map(|(name, &list)| (name.clone(), arena.copy(list)))
                .collect(),
            _state: PhantomData,
        }
    }
}

impl FetchStepSelections<MultiTypeFetchStep> {
    /// Meant for input and output of entity calls.
    pub fn to_non_root_selection_set(&self, arena: &Arena) -> SelectionSet {
        self.wrap_in_type_fragments(arena)
    }

    pub fn to_root_selection_set(&self, arena: &Arena, root_type_name: &str) -> SelectionSet {
        if self.selections.len() == 1 {
            let (type_name, &selections) = self.selections.iter().next().unwrap();

            if type_name == root_type_name {
                return arena.export(selections);
            }
        }

        self.wrap_in_type_fragments(arena)
    }

    fn wrap_in_type_fragments(&self, arena: &Arena) -> SelectionSet {
        SelectionSet {
            items: self
                .selections
                .iter()
                .map(|(type_name, &list)| {
                    let selections = arena.export(list);
                    // Build one `... on Type` wrapper item if needed
                    let Some((condition, selections_for_wrapper)) =
                        try_lift_condition(type_name, &selections)
                    else {
                        return SelectionItem::InlineFragment(InlineFragmentSelection {
                            type_condition: type_name.to_string(),
                            include_if: None,
                            skip_if: None,
                            selections,
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
    arena: &Arena,
    type_name: &str,
    list: ListId,
) -> Option<Condition> {
    let mut condition = None;

    for &item in arena.items(list) {
        let node = arena.node(item);
        if node.kind != Kind::InlineFragment || arena.str(node.name) != type_name {
            return None;
        }

        let fragment_condition = arena.condition(item)?;

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
fn clear_top_level_fragment_conditions(arena: &mut Arena, type_name: &str, list: ListId) {
    for index in 0..arena.items(list).len() {
        let item = arena.items(list)[index];
        let node = arena.node(item);
        if node.kind == Kind::InlineFragment && arena.str(node.name) == type_name {
            arena.set_condition(item, None);
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

impl FetchStepSelections<SingleTypeFetchStep> {
    /// The same lists, as a multi-type step's. Only for when this one goes away.
    pub(crate) fn move_into_multi_type(&self) -> FetchStepSelections<MultiTypeFetchStep> {
        FetchStepSelections {
            _state: Default::default(),
            selections: self.selections.clone(),
        }
    }

    pub fn definition_name(&self) -> &str {
        self.first_definition_name()
            .expect("SingleTypeFetchStep should have exactly one selection")
    }

    pub fn selection_set(&self) -> ListId {
        *self
            .selections
            .values()
            .next()
            .expect("SingleTypeFetchStep should have exactly one selection")
    }

    pub fn add_at_path(
        &mut self,
        arena: &mut Arena,
        fetch_path: &MergePath,
        selection_set: SelectionSet,
    ) -> Result<(), FetchStepSelectionsError> {
        let def_name = self.definition_name().to_string();
        let source = arena.import(&selection_set);

        self.add_at_path_inner(arena, &def_name, fetch_path, source, false)
    }

    pub fn add_selection_typename(
        &mut self,
        arena: &mut Arena,
        fetch_path: &MergePath,
    ) -> Result<(), FetchStepSelectionsError> {
        let def_name = self.definition_name().to_string();
        let source = arena.import(&SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection::new_typename())],
        });

        self.add_at_path_inner(arena, &def_name, fetch_path, source, true)
    }
}

impl<State> FetchStepSelections<State> {
    /// The first type name, which on a single-type step is its only one.
    pub fn first_definition_name(&self) -> Option<&str> {
        self.selections.keys().next().map(String::as_str)
    }

    pub fn is_fetching_multiple_types(&self) -> bool {
        self.selections.len() > 1
    }

    pub fn contains(
        &self,
        arena: &Arena,
        definition_name: &str,
        selection_set: &SelectionSet,
    ) -> bool {
        if let Some(&self_selections) = self.selections.get(definition_name) {
            return arena.contains_tree(self_selections, selection_set);
        }

        false
    }

    pub fn is_selecting_definition(&self, definition_name: &str) -> bool {
        self.selections.contains_key(definition_name)
    }

    pub fn is_empty(&self, arena: &Arena) -> bool {
        self.selections.is_empty() || self.selections.values().all(|&list| arena.is_empty(list))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, ListId)> {
        self.selections.iter().map(|(name, &list)| (name, list))
    }

    pub fn variable_usages(&self, arena: &Arena) -> BTreeSet<String> {
        let mut usages = BTreeSet::new();

        for &list in self.selections.values() {
            arena.variable_usages(list, &mut usages);
        }

        usages
    }

    /// Puts `list` in place of the selections for `definition_name`, if it has some.
    pub fn set_selections_for_definition(&mut self, definition_name: &str, list: ListId) {
        if let Some(current) = self.selections.get_mut(definition_name) {
            *current = list;
        }
    }

    pub fn selections_for_definition(&self, definition_name: &str) -> Option<ListId> {
        self.selections.get(definition_name).copied()
    }

    fn add_at_path_inner(
        &mut self,
        arena: &mut Arena,
        definition_name: &str,
        fetch_path: &MergePath,
        source: ListId,
        as_first: bool,
    ) -> Result<(), FetchStepSelectionsError> {
        let current = self
            .selections_for_definition(definition_name)
            .ok_or_else(|| {
                FetchStepSelectionsError::UnexpectedMissingDefinition(definition_name.to_string())
            })?;

        let selection_set_at_path = arena.find_by_path(current, fetch_path).ok_or_else(|| {
            FetchStepSelectionsError::MissingPathInSelection(
                fetch_path.to_string(),
                definition_name.to_string(),
            )
        })?;

        arena.merge(selection_set_at_path, source, as_first)?;

        Ok(())
    }
}

impl FetchStepSelections<MultiTypeFetchStep> {
    fn wrap_definition_selection_with_condition(
        arena: &mut Arena,
        def_name: &str,
        list: ListId,
        condition: &Condition,
    ) {
        // A type slot can be declared before it gets any selections (multi-type batching does
        // that right before a merge). Nothing to guard there, and `... on T @include { }` is
        // invalid GraphQL that ends up in the subgraph request.
        if arena.is_empty(list) {
            return;
        }
        let fragment = arena.new_fragment(def_name, Some(condition));
        let fragment_list = arena.node(fragment).children;
        let items = arena.items(list).to_vec();
        arena.set_items(fragment_list, &items);
        arena.set_items(list, &[fragment]);
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
    pub fn response_keys_conflict_with(&self, arena: &Arena, other: &Self) -> bool {
        self.iter_selections().any(|(definition_name, selections)| {
            other
                .selections
                .get(definition_name)
                .is_some_and(|&other_selections| {
                    arena.response_keys_conflict(selections, other_selections)
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

    pub fn iter_selections(&self) -> impl Iterator<Item = (&String, ListId)> {
        self.iter()
    }

    /// Creates a slot in the internal HashMap and will allow to add selections for the given definition name.
    /// Without that, trying to add selections using any function will either fail or result in trying to force-add to a root type.
    /// Calling this method is crucial if you wish to create multi-type steps.
    pub fn declare_known_type(&mut self, arena: &mut Arena, def_name: &str) {
        if !self.selections.contains_key(def_name) {
            self.selections
                .insert(def_name.to_string(), arena.new_list());
        }
    }

    /// A step's selections from its lists, by type name. The lists become its own.
    pub fn from_lists(lists: impl IntoIterator<Item = (String, ListId)>) -> Self {
        Self {
            selections: lists.into_iter().collect(),
            _state: PhantomData,
        }
    }

    pub fn migrate_from_another(
        &mut self,
        arena: &mut Arena,
        other: &Self,
        fetch_path: &MergePath,
    ) -> Result<(), FetchStepSelectionsError> {
        let maybe_merge_into = self.try_as_single().map(|str| str.to_string());

        for (definition_name, selection_set) in other.iter_selections() {
            let target_type = maybe_merge_into.as_ref().unwrap_or(definition_name);
            self.add_at_path_inner(arena, target_type, fetch_path, selection_set, false)?;
        }

        Ok(())
    }

    pub fn wrap_with_condition(&mut self, arena: &mut Arena, condition: Condition) {
        for (def_name, &list) in self.selections.iter() {
            Self::wrap_definition_selection_with_condition(arena, def_name, list, &condition);
        }
    }

    pub fn wrap_with_condition_for_types(
        &mut self,
        arena: &mut Arena,
        condition: Condition,
        type_names: &BTreeSet<&str>,
    ) {
        for (def_name, &list) in self.selections.iter() {
            if type_names.contains(def_name.as_str()) {
                Self::wrap_definition_selection_with_condition(arena, def_name, list, &condition);
            }
        }
    }

    // Tries to lift a shared output condition into the fetch step.
    // A step-level condition means the whole fetch can be skipped.
    // `Some` only when every type is guarded by the same top-level condition.
    // The lifted wrapper directives are cleared from the output
    // to avoid duplicating the same condition at both fetch and selection level.
    pub fn take_shared_top_level_fragment_condition(
        &mut self,
        arena: &mut Arena,
    ) -> Option<Condition> {
        if self.is_empty(arena) {
            return None;
        }

        let mut common_condition = None;

        for (type_name, list) in self.iter() {
            let condition = shared_top_level_fragment_condition(arena, type_name, list)?;

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
        for (type_name, &list) in self.selections.iter() {
            clear_top_level_fragment_conditions(arena, type_name, list);
        }

        Some(condition)
    }
}

impl FetchStepSelections<SingleTypeFetchStep> {
    pub fn add(
        &mut self,
        arena: &mut Arena,
        selection_set: &SelectionSet,
    ) -> Result<(), FetchStepSelectionsError> {
        let source = arena.import(selection_set);
        arena.merge(self.selection_set(), source, false)?;

        Ok(())
    }

    pub fn new(arena: &mut Arena, definition_name: &str) -> Self {
        let mut map = BTreeMap::new();
        map.insert(definition_name.to_string(), arena.new_list());

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
    use crate::query_planner::ast::flat::arena::Arena;

    fn parse_selection_set(input: &str) -> SelectionSet {
        let op = parse_operation(input);

        match op.definitions.first() {
            Some(Definition::Operation(OperationDefinition::SelectionSet(s))) => s.clone().into(),
            _ => panic!("expected top-level selection set input"),
        }
    }

    fn multi_type_from_top_level_inline_fragments(
        arena: &mut Arena,
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
            selections: map
                .into_iter()
                .map(|(name, set)| (name, arena.import(&set)))
                .collect(),
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
        let mut arena = Arena::new();
        let fetch_selections = multi_type_from_top_level_inline_fragments(
            &mut arena,
            r#"
            {
              ... on Book @skip(if: $title) {
                sku
              }
            }
            "#,
        );
        insta::assert_snapshot!(
            format!("{}", PrettySelectionSet(fetch_selections.to_non_root_selection_set(&arena))),
            @r#"
              ... on Book @skip(if: $title) {
                sku
              }
            "#
        );
    }

    #[test]
    fn lifts_shared_condition_from_multiple_fragments() {
        let mut arena = Arena::new();
        let fetch_selections = multi_type_from_top_level_inline_fragments(
            &mut arena,
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
          format!("{}", PrettySelectionSet(fetch_selections.to_non_root_selection_set(&arena))),
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
        let mut arena = Arena::new();
        let fetch_selections = multi_type_from_top_level_inline_fragments(
            &mut arena,
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
          format!("{}", PrettySelectionSet(fetch_selections.to_non_root_selection_set(&arena))),
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
        let mut arena = Arena::new();
        let fetch_selections = multi_type_from_top_level_inline_fragments(
            &mut arena,
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
          format!("{}", PrettySelectionSet(fetch_selections.to_non_root_selection_set(&arena))),
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
