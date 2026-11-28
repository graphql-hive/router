use graphql_tools::parser::query as query_ast;
use serde::{ser::SerializeSeq, Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt::{Debug, Display},
    hash::Hash,
};

use crate::query_planner::{
    ast::merge_path::{Condition, MergePath, Segment},
    ast::value::Value,
    utils::pretty_display::{get_indent, PrettyDisplay},
};

use super::{arguments::ArgumentsMap, selection_item::SelectionItem, shrink::ShrinkMemory};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SelectionSet {
    pub items: Vec<SelectionItem>,
}

impl<'a, T: query_ast::Text<'a>> From<query_ast::SelectionSet<'a, T>> for SelectionSet {
    fn from(selection_set: query_ast::SelectionSet<'a, T>) -> Self {
        let mut items = Vec::with_capacity(selection_set.items.len());
        items.extend(selection_set.items.into_iter().map(SelectionItem::from));
        Self { items }
    }
}

impl PartialEq for SelectionSet {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl Eq for SelectionSet {}

impl Display for SelectionSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.items.is_empty() {
            return Ok(());
        }

        write!(f, "{{")?;
        for (i, item) in self.items.iter().enumerate() {
            if i + 1 == self.items.len() {
                write!(f, "{}", item)?;
            } else {
                write!(f, "{} ", item)?;
            }
        }
        write!(f, "}}")?;
        Ok(())
    }
}

impl ShrinkMemory for SelectionSet {
    fn shrink_memory(&mut self) {
        self.items.shrink_memory();
    }
}

impl SelectionSet {
    pub fn cost(&self) -> u64 {
        let mut cost = 1;

        for node in &self.items {
            cost += node.cost();
        }

        cost
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, other: &Self) -> bool {
        selection_items_are_subset_of(&self.items, &other.items)
    }

    pub fn variable_usages(&self) -> BTreeSet<String> {
        self.items
            .iter()
            .flat_map(|item| item.variable_usages())
            .collect()
    }

    pub fn strip_for_plan_input(&self) -> Self {
        SelectionSet {
            items: self
                .items
                .iter()
                .map(|item| item.strip_for_plan_input())
                .collect(),
        }
    }

    pub fn entities_field(&self) -> Option<&FieldSelection> {
        self.items.iter().find_map(|item| {
            if let SelectionItem::Field(field) = item {
                if field.name == "_entities" {
                    return Some(field);
                }
            }
            None
        })
    }

    pub fn iter_fields_and_fragments_of_same_type<'a>(
        &'a self,
        type_name: &'a str,
    ) -> impl Iterator<Item = &'a FieldSelection> + 'a {
        let mut stack = vec![self.items.iter()];

        std::iter::from_fn(move || loop {
            let iter = stack.last_mut()?;

            match iter.next() {
                Some(SelectionItem::Field(field)) => {
                    return Some(field);
                }

                Some(SelectionItem::InlineFragment(fragment))
                    if fragment.type_condition == type_name =>
                {
                    stack.push(fragment.selections.items.iter());
                }

                Some(_) => {}

                None => {
                    stack.pop();
                }
            }
        })
    }
}

impl Hash for SelectionSet {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.items.hash(state);
    }
}

impl Serialize for SelectionSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.items.len()))?;
        for e in &self.items {
            seq.serialize_element(&e)?;
        }
        seq.end()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FieldSelection {
    pub name: String,
    #[serde(skip_serializing_if = "SelectionSet::is_empty")]
    pub selections: SelectionSet,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<ArgumentsMap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_if: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_if: Option<String>,
    /// Whether to skip this field in response projection.
    /// Defaults to `false`.
    /// The only case so far for it is when the selection set becomes empty,
    /// due to a field being `@skip(if: true)` or `@include(if: false)`,
    /// and used to avoid sending empty selection sets,
    /// and to project an empty object.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub omit_from_response: bool,
}

impl Hash for FieldSelection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);

        if let Some(alias) = &self.alias {
            alias.hash(state);
        }

        self.selections.hash(state);

        if let Some(arguments) = &self.arguments {
            arguments.hash(state);
        }
    }
}

impl PartialEq for FieldSelection {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.alias == other.alias
            && self.arguments() == other.arguments()
    }
}

impl FieldSelection {
    /// Whether it's `__typename`.
    #[inline]
    pub fn is_typename(&self) -> bool {
        self.name == "__typename"
    }

    pub fn with_new_selections(&self, selections: SelectionSet) -> Self {
        FieldSelection {
            name: self.name.clone(),
            alias: self.alias.clone(),
            selections,
            arguments: self.arguments.clone(),
            skip_if: self.skip_if.clone(),
            include_if: self.include_if.clone(),
            omit_from_response: self.omit_from_response,
        }
    }

    /// Returns the unique identifier of the field within the selection set.
    /// This means, the alias or the field name if no alias is present.
    pub fn selection_identifier(&self) -> &str {
        match &self.alias {
            Some(alias) => alias,
            None => &self.name,
        }
    }

    /// Calculates a hash value based on the arguments of the field selection.
    /// If no arguments are present, returns 0.
    /// This is used to determine if two field selections are equal, and to avoid conflicts in the selection sets we produce.
    pub fn arguments_hash(&self) -> u64 {
        if let Some(arguments) = &self.arguments {
            return arguments.hash_u64();
        }

        0
    }

    pub fn is_leaf(&self) -> bool {
        self.selections.is_empty()
    }

    pub fn new_typename() -> Self {
        FieldSelection {
            name: "__typename".to_string(),
            alias: None,
            selections: SelectionSet::default(),
            arguments: None,
            skip_if: None,
            include_if: None,
            omit_from_response: false,
        }
    }

    // Returns a field selection that skips the `__typename` field in the response.
    pub fn new_skipped_typename() -> Self {
        FieldSelection {
            name: "__typename".to_string(),
            alias: None,
            selections: SelectionSet::default(),
            arguments: None,
            skip_if: None,
            include_if: None,
            omit_from_response: true,
        }
    }

    pub fn variable_usages(&self) -> BTreeSet<String> {
        let mut usages = BTreeSet::new();

        if let Some(arguments) = &self.arguments {
            for value in arguments.values() {
                usages.extend(value.variable_usages());
            }
        }

        if let Some(include_if) = &self.include_if {
            usages.insert(include_if.clone());
        }

        if let Some(skip_if) = &self.skip_if {
            usages.insert(skip_if.clone());
        }

        usages.extend(self.selections.variable_usages());
        usages
    }

    pub fn arguments(&self) -> Option<&ArgumentsMap> {
        match &self.arguments {
            Some(arguments) => {
                if arguments.is_empty() {
                    None
                } else {
                    Some(arguments)
                }
            }
            None => None,
        }
    }

    pub fn is_introspection_field(&self) -> bool {
        self.name.starts_with("__")
    }

    /// Returns the name of the variable if this field represents a representation variable.
    pub fn representations_variable_name(&self) -> Option<&str> {
        let value = self.arguments.as_ref()?.get_argument("representations")?;

        match value {
            Value::Variable(variable_name) => Some(variable_name.as_str()),
            _ => None,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineFragmentSelection {
    pub type_condition: String,
    pub selections: SelectionSet,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_if: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_if: Option<String>,
}

impl InlineFragmentSelection {
    pub fn variable_usages(&self) -> BTreeSet<String> {
        let mut usages = BTreeSet::new();

        if let Some(include_if) = &self.include_if {
            usages.insert(include_if.clone());
        }

        if let Some(skip_if) = &self.skip_if {
            usages.insert(skip_if.clone());
        }

        usages.extend(self.selections.variable_usages());
        usages
    }

    pub fn with_new_selections(&self, selections: SelectionSet) -> Self {
        InlineFragmentSelection {
            type_condition: self.type_condition.clone(),
            selections,
            skip_if: self.skip_if.clone(),
            include_if: self.include_if.clone(),
        }
    }
}

impl Hash for InlineFragmentSelection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_condition.hash(state);
        self.selections.hash(state);
    }
}

impl Display for FieldSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(alias) = &self.alias {
            write!(f, "{}: ", alias)?;
        }

        write!(f, "{}", self.name)?;

        if let Some(arguments) = &self.arguments() {
            write!(f, "({})", arguments)?;
        }

        if let Some(skip_if) = &self.skip_if {
            write!(f, " @skip(if: ${})", skip_if)?;
        }

        if let Some(include_if) = &self.include_if {
            write!(f, " @include(if: ${})", include_if)?;
        }

        write!(f, "{}", self.selections)
    }
}

impl PrettyDisplay for FieldSelection {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);

        let alias_str = match &self.alias {
            Some(alias_name) => format!("{}: ", alias_name),
            None => String::new(),
        };

        let args_str = match &self.arguments() {
            Some(arguments) => format!("({})", arguments),
            None => String::new(),
        };

        write!(f, "{indent}{}{}{}", alias_str, self.name, args_str)?;

        if let Some(skip_if) = &self.skip_if {
            write!(f, " @skip(if: ${})", skip_if)?;
        }

        if let Some(include_if) = &self.include_if {
            write!(f, " @include(if: ${})", include_if)?;
        }

        if self.is_leaf() {
            return writeln!(f);
        }

        writeln!(f, " {{")?;
        self.selections.pretty_fmt(f, depth + 1)?;
        writeln!(f, "{indent}}}")
    }
}

impl PrettyDisplay for SelectionSet {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        for item in self.items.iter() {
            item.pretty_fmt(f, depth)?;
        }

        Ok(())
    }
}

impl Display for InlineFragmentSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "...on {}", self.type_condition)?;
        if let Some(skip_if) = &self.skip_if {
            write!(f, " @skip(if: ${})", skip_if)?;
        }
        if let Some(include_if) = &self.include_if {
            write!(f, " @include(if: ${})", include_if)?;
        }
        write!(f, "{}", self.selections)
    }
}

impl PrettyDisplay for InlineFragmentSelection {
    fn pretty_fmt(&self, f: &mut std::fmt::Formatter<'_>, depth: usize) -> std::fmt::Result {
        let indent = get_indent(depth);

        write!(f, "{indent}... on {} ", self.type_condition)?;
        if let Some(skip_if) = &self.skip_if {
            write!(f, "@skip(if: ${}) ", skip_if)?;
        }
        if let Some(include_if) = &self.include_if {
            write!(f, "@include(if: ${}) ", include_if)?;
        }

        writeln!(f, "{{")?;

        self.selections.pretty_fmt(f, depth + 1)?;
        writeln!(f, "{indent}}}")
    }
}

pub fn selection_items_are_subset_of(source: &[SelectionItem], target: &[SelectionItem]) -> bool {
    target.iter().all(|target_node| {
        source
            .iter()
            .any(|source_node| selection_item_is_subset_of(source_node, target_node))
    })
}

/// Whether `source` gives the same value `target` asks for. Same response key, field and
/// arguments, and the same `@skip`/`@include`: `price(currency: "GBP")` isn't
/// `price(currency: "EUR")`, and a field fetched only under `$x` isn't there under `$y`.
/// Conditions have to be equal, not just implied, which is stricter than needed but never wrong.
fn selection_item_is_subset_of(source: &SelectionItem, target: &SelectionItem) -> bool {
    match (source, target) {
        (SelectionItem::Field(source_field), SelectionItem::Field(target_field)) => {
            source_field == target_field
                && source_field.skip_if == target_field.skip_if
                && source_field.include_if == target_field.include_if
                && source_field.is_leaf() == target_field.is_leaf()
                && selection_items_are_subset_of(
                    &source_field.selections.items,
                    &target_field.selections.items,
                )
        }
        (
            SelectionItem::InlineFragment(source_fragment),
            SelectionItem::InlineFragment(target_fragment),
        ) => {
            source_fragment.type_condition == target_fragment.type_condition
                && source_fragment.skip_if == target_fragment.skip_if
                && source_fragment.include_if == target_fragment.include_if
                && selection_items_are_subset_of(
                    &source_fragment.selections.items,
                    &target_fragment.selections.items,
                )
        }
        _ => false,
    }
}

/// Two different fields under one response key, on the same object.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Response key '{0}' would hold two different fields")]
pub struct ResponseKeyConflict(pub String);

/// Merges `source` into `target`. Fails when a field of `source` lands next to another field
/// with the same response key but another name or arguments: the object can't hold both.
/// Fields in different fragments are never merged together, so they can't meet here.
pub fn merge_selection_set(
    target: &mut SelectionSet,
    source: &SelectionSet,
    as_first: bool,
) -> Result<(), ResponseKeyConflict> {
    if source.items.is_empty() {
        return Ok(());
    }

    let mut pending_items = Vec::with_capacity(source.items.len());
    for source_item in source.items.iter() {
        let Some(index) = merge_target(&target.items, source_item)? else {
            pending_items.push(source_item.clone());
            continue;
        };
        match (source_item, &mut target.items[index]) {
            (SelectionItem::Field(source_field), SelectionItem::Field(target_field)) => {
                merge_field_omit_from_response(target_field, source_field);
                merge_selection_set(
                    &mut target_field.selections,
                    &source_field.selections,
                    as_first,
                )?;
            }
            (
                SelectionItem::InlineFragment(source_fragment),
                SelectionItem::InlineFragment(target_fragment),
            ) => {
                merge_selection_set(
                    &mut target_fragment.selections,
                    &source_fragment.selections,
                    as_first,
                )?;
            }
            _ => {}
        }
    }
    add_unmerged(target, pending_items, as_first);
    Ok(())
}

/// Like `merge_selection_set`, moving what `source` has instead of copying it.
pub fn merge_selection_set_owned(
    target: &mut SelectionSet,
    source: SelectionSet,
    as_first: bool,
) -> Result<(), ResponseKeyConflict> {
    if source.items.is_empty() {
        return Ok(());
    }

    let mut pending_items = Vec::with_capacity(source.items.len());
    for source_item in source.items {
        let Some(index) = merge_target(&target.items, &source_item)? else {
            pending_items.push(source_item);
            continue;
        };
        match (source_item, &mut target.items[index]) {
            (SelectionItem::Field(mut source_field), SelectionItem::Field(target_field)) => {
                merge_field_omit_from_response(target_field, &source_field);
                merge_selection_set_owned(
                    &mut target_field.selections,
                    std::mem::take(&mut source_field.selections),
                    as_first,
                )?;
            }
            (
                SelectionItem::InlineFragment(source_fragment),
                SelectionItem::InlineFragment(target_fragment),
            ) => {
                merge_selection_set_owned(
                    &mut target_fragment.selections,
                    source_fragment.selections,
                    as_first,
                )?;
            }
            _ => {}
        }
    }
    add_unmerged(target, pending_items, as_first);
    Ok(())
}

/// Merges `source` into a mutation's root, `target`. Its fields run in order, so a fragment only
/// merges into the item right before it, and the rest goes after. Fragments in there are still
/// the root, so it carries into them; a field's selections merge as usual.
pub fn merge_mutation_root(
    target: &mut SelectionSet,
    source: SelectionSet,
) -> Result<(), ResponseKeyConflict> {
    for source_item in source.items {
        match (source_item, target.items.last_mut()) {
            (
                SelectionItem::InlineFragment(source_fragment),
                Some(SelectionItem::InlineFragment(last)),
            ) if source_fragment.type_condition == last.type_condition
                && fragment_condition_equal(&Option::<Condition>::from(&source_fragment), last) =>
            {
                merge_mutation_root(&mut last.selections, source_fragment.selections)?;
            }
            (SelectionItem::Field(mut source_field), _) => {
                // the same field under the same condition runs once, at its first spot
                match merge_target(&target.items, &SelectionItem::Field(source_field.clone()))? {
                    Some(index) => {
                        if let SelectionItem::Field(target_field) = &mut target.items[index] {
                            merge_field_omit_from_response(target_field, &source_field);
                            merge_selection_set_owned(
                                &mut target_field.selections,
                                std::mem::take(&mut source_field.selections),
                                false,
                            )?;
                        }
                    }
                    None => target.items.push(SelectionItem::Field(source_field)),
                }
            }
            (source_item, _) => target.items.push(source_item),
        }
    }
    Ok(())
}

/// The item of `target_items` that `source_item` merges into: the same field with the same
/// condition, or a fragment on the same type with the same condition. An error when a field
/// there has its response key but is another field.
fn merge_target(
    target_items: &[SelectionItem],
    source_item: &SelectionItem,
) -> Result<Option<usize>, ResponseKeyConflict> {
    for (index, target_item) in target_items.iter().enumerate() {
        match (source_item, target_item) {
            (SelectionItem::Field(source_field), SelectionItem::Field(target_field))
                if source_field.selection_identifier() == target_field.selection_identifier()
                    && (source_field.name != target_field.name
                        || source_field.arguments() != target_field.arguments()) =>
            {
                return Err(ResponseKeyConflict(
                    source_field.selection_identifier().to_string(),
                ));
            }
            (SelectionItem::Field(source_field), SelectionItem::Field(target_field))
                if source_field == target_field
                    && field_condition_equal(
                        &Option::<Condition>::from(source_field),
                        target_field,
                    ) =>
            {
                return Ok(Some(index));
            }
            (
                SelectionItem::InlineFragment(source_fragment),
                SelectionItem::InlineFragment(target_fragment),
            ) if source_fragment.type_condition == target_fragment.type_condition
                && fragment_condition_equal(
                    &Option::<Condition>::from(source_fragment),
                    target_fragment,
                ) =>
            {
                return Ok(Some(index));
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Puts the items nothing in `target` took before or after its own.
fn add_unmerged(target: &mut SelectionSet, mut items: Vec<SelectionItem>, as_first: bool) {
    if items.is_empty() {
        return;
    }
    if as_first {
        items.append(&mut target.items);
        target.items = items;
    } else {
        target.items.extend(items);
    }
}

#[inline]
fn merge_field_omit_from_response(target: &mut FieldSelection, source: &FieldSelection) {
    // A skipped `__typename` is only there for an empty object.
    // If the client explicitly requested `__typename`, keep the visible field.
    if target.is_typename() {
        target.omit_from_response &= source.omit_from_response;
    }
}

/// Does this path segment point at this selection item?
fn segment_selects(segment: &Segment, item: &SelectionItem) -> bool {
    match (segment, item) {
        (
            Segment::TypeCondition(type_names, condition),
            SelectionItem::InlineFragment(fragment),
        ) => {
            type_names.contains(&fragment.type_condition)
                && fragment_condition_equal(condition, fragment)
        }
        (Segment::Field(field_seg, args_hash, condition), SelectionItem::Field(field)) => {
            field.selection_identifier() == field_seg.response_key()
                && field.arguments_hash() == *args_hash
                && field_condition_equal(condition, field)
        }
        _ => false,
    }
}

fn item_selections(item: &SelectionItem) -> Option<&SelectionSet> {
    match item {
        SelectionItem::Field(field) => Some(&field.selections),
        SelectionItem::InlineFragment(fragment) => Some(&fragment.selections),
        SelectionItem::FragmentSpread(_) => None,
    }
}

fn item_selections_mut(item: &mut SelectionItem) -> Option<&mut SelectionSet> {
    match item {
        SelectionItem::Field(field) => Some(&mut field.selections),
        SelectionItem::InlineFragment(fragment) => Some(&mut fragment.selections),
        SelectionItem::FragmentSpread(_) => None,
    }
}

pub fn find_selection_set_by_path<'a>(
    root_selection_set: &'a SelectionSet,
    path: &MergePath,
) -> Option<&'a SelectionSet> {
    find_selection_set_by_segments(root_selection_set, &path.inner)
}

/// `find_selection_set_by_path` with a part of a path. A segment can match more than one item,
/// like two `... on Mutation @include(if: $on)` kept apart so the fields between them keep their
/// order: it goes into the first one the rest of the path is under.
pub fn find_selection_set_by_segments<'a>(
    selection_set: &'a SelectionSet,
    path: &[Segment],
) -> Option<&'a SelectionSet> {
    let Some((segment, rest)) = path.split_first() else {
        return Some(selection_set);
    };
    if matches!(segment, Segment::List) {
        return find_selection_set_by_segments(selection_set, rest);
    }
    selection_set
        .items
        .iter()
        .filter(|item| segment_selects(segment, item))
        .filter_map(item_selections)
        .find_map(|selections| find_selection_set_by_segments(selections, rest))
}

pub fn find_selection_set_by_path_mut<'a>(
    root_selection_set: &'a mut SelectionSet,
    path: &MergePath,
) -> Option<&'a mut SelectionSet> {
    find_selection_set_by_segments_mut(root_selection_set, &path.inner)
}

/// `find_selection_set_by_segments`, mutable.
fn find_selection_set_by_segments_mut<'a>(
    selection_set: &'a mut SelectionSet,
    path: &[Segment],
) -> Option<&'a mut SelectionSet> {
    let Some((segment, rest)) = path.split_first() else {
        return Some(selection_set);
    };
    if matches!(segment, Segment::List) {
        return find_selection_set_by_segments_mut(selection_set, rest);
    }
    selection_set
        .items
        .iter_mut()
        .filter(|item| segment_selects(segment, item))
        .filter_map(item_selections_mut)
        .find_map(|selections| find_selection_set_by_segments_mut(selections, rest))
}

/// Like `find_selection_set_by_path_mut`, but a type condition with several types, like
/// `product|[Book|Movie]`, goes into the fragment of each, so it finds one selection set for each
/// type. The fragments are different items, so they're found in one walk, in the order they're
/// in. `false` when one of them isn't there.
pub fn find_selection_sets_by_path_mut<'a>(
    selection_set: &'a mut SelectionSet,
    path: &[Segment],
    found: &mut Vec<&'a mut SelectionSet>,
) -> bool {
    let Some((segment, rest)) = path.split_first() else {
        found.push(selection_set);
        return true;
    };
    match segment {
        Segment::List => find_selection_sets_by_path_mut(selection_set, rest, found),
        Segment::TypeCondition(type_names, condition) if type_names.len() > 1 => {
            // the first fragment of each type, like a path with only that type would find
            let mut taken: Vec<&str> = Vec::with_capacity(type_names.len());
            for item in selection_set.items.iter_mut() {
                let SelectionItem::InlineFragment(fragment) = item else {
                    continue;
                };
                let Some(type_name) = type_names.get(&fragment.type_condition) else {
                    continue;
                };
                if taken.contains(&type_name.as_str())
                    || !fragment_condition_equal(condition, fragment)
                {
                    continue;
                }
                taken.push(type_name.as_str());
                if !find_selection_sets_by_path_mut(&mut fragment.selections, rest, found) {
                    return false;
                }
            }
            taken.len() == type_names.len()
        }
        // the first item the rest of the path is under, see `find_selection_set_by_segments`
        segment => {
            let before = found.len();
            for item in selection_set.items.iter_mut() {
                if !segment_selects(segment, item) {
                    continue;
                }
                let Some(selections) = item_selections_mut(item) else {
                    continue;
                };
                if find_selection_sets_by_path_mut(selections, rest, found) {
                    return true;
                }
                found.truncate(before);
            }
            false
        }
    }
}

pub fn field_condition_equal(cond: &Option<Condition>, field: &FieldSelection) -> bool {
    match cond {
        Some(cond) => match cond {
            Condition::Include(var_name) => {
                field.include_if.as_ref().is_some_and(|v| v == var_name) && field.skip_if.is_none()
            }
            Condition::Skip(var_name) => {
                field.skip_if.as_ref().is_some_and(|v| v == var_name) && field.include_if.is_none()
            }
            Condition::SkipAndInclude { skip, include } => {
                field.skip_if.as_ref().is_some_and(|v| v == skip)
                    && field.include_if.as_ref().is_some_and(|v| v == include)
            }
        },
        None => field.include_if.is_none() && field.skip_if.is_none(),
    }
}

fn fragment_condition_equal(cond: &Option<Condition>, fragment: &InlineFragmentSelection) -> bool {
    match cond {
        Some(cond) => match cond {
            Condition::Include(var_name) => {
                fragment.include_if.as_ref().is_some_and(|v| v == var_name)
                    && fragment.skip_if.is_none()
            }
            Condition::Skip(var_name) => {
                fragment.skip_if.as_ref().is_some_and(|v| v == var_name)
                    && fragment.include_if.is_none()
            }
            Condition::SkipAndInclude { skip, include } => {
                fragment.skip_if.as_ref().is_some_and(|v| v == skip)
                    && fragment.include_if.as_ref().is_some_and(|v| v == include)
            }
        },
        None => fragment.include_if.is_none() && fragment.skip_if.is_none(),
    }
}

/// Whether `a` and `b` in one selection set would give a response key two values: the same key
/// for another field or other arguments, at any depth. Inline fragments are looked through,
/// their fields write the same object.
pub fn response_keys_conflict(a: &SelectionSet, b: &SelectionSet) -> bool {
    any_field_through_fragments(a, &|a_field| {
        any_field_through_fragments(b, &|b_field| {
            a_field.selection_identifier() == b_field.selection_identifier()
                && (a_field.name != b_field.name
                    || a_field.arguments() != b_field.arguments()
                    || response_keys_conflict(&a_field.selections, &b_field.selections))
        })
    })
}

/// Like `fields_through_fragments`, without collecting them, so the check runs on every merge
/// candidate without allocating.
fn any_field_through_fragments(
    selection_set: &SelectionSet,
    predicate: &impl Fn(&FieldSelection) -> bool,
) -> bool {
    selection_set.items.iter().any(|item| match item {
        SelectionItem::Field(field) => predicate(field),
        SelectionItem::InlineFragment(fragment) => {
            any_field_through_fragments(&fragment.selections, predicate)
        }
        SelectionItem::FragmentSpread(_) => false,
    })
}

/// The fields of an object: the ones in `selection_set` and in its inline fragments, at any depth.
pub fn fields_through_fragments(selection_set: &SelectionSet) -> Vec<&FieldSelection> {
    fn collect<'a>(selection_set: &'a SelectionSet, fields: &mut Vec<&'a FieldSelection>) {
        for item in &selection_set.items {
            match item {
                SelectionItem::Field(field) => fields.push(field),
                SelectionItem::InlineFragment(fragment) => collect(&fragment.selections, fields),
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    let mut fields = Vec::new();
    collect(selection_set, &mut fields);
    fields
}

#[cfg(test)]
mod tests {
    use crate::query_planner::ast::value::Value;

    use super::*;

    #[test]
    fn print_alias_selection_set() {
        let selection_set = SelectionSet {
            items: vec![
                SelectionItem::Field(FieldSelection {
                    name: "field1".to_string(),
                    selections: SelectionSet::default(),
                    alias: Some("f".to_string()),
                    arguments: None,
                    skip_if: None,
                    include_if: None,
                    omit_from_response: false,
                }),
                SelectionItem::Field(FieldSelection {
                    name: "field2".to_string(),
                    selections: SelectionSet {
                        items: vec![SelectionItem::Field(FieldSelection {
                            name: "nested".to_string(),
                            selections: SelectionSet::default(),
                            alias: Some("n".to_string()),
                            arguments: Some(("a".to_string(), Value::Int(1)).into()),
                            skip_if: None,
                            include_if: None,
                            omit_from_response: false,
                        })],
                    },
                    alias: Some("f2".to_string()),
                    arguments: None,
                    skip_if: None,
                    include_if: None,
                    omit_from_response: false,
                }),
            ],
        };

        insta::assert_snapshot!(
          selection_set,
          @"{f: field1 f2: field2{n: nested(a: 1)}}"
        )
    }

    #[test]
    fn print_simple_selection_set() {
        let selection_set = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "field1".to_string(),
                selections: SelectionSet::default(),
                alias: None,
                arguments: None,
                skip_if: None,
                include_if: None,
                omit_from_response: false,
            })],
        };

        insta::assert_snapshot!(
          selection_set,
          @"{field1}"
        )
    }

    #[test]
    fn selection_set_with_arguments() {
        let selection_set = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "field1".to_string(),
                selections: SelectionSet::default(),
                alias: None,
                arguments: Some(vec![("id".to_string(), Value::Int(1))].into()),
                skip_if: None,
                include_if: None,
                omit_from_response: false,
            })],
        };

        insta::assert_snapshot!(
          selection_set,
          @"{field1(id: 1)}"
        )
    }

    #[test]
    fn complex_selection_set() {
        let selection_set = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "field1".to_string(),
                selections: SelectionSet::default(),
                alias: None,
                arguments: Some(
                    vec![
                        ("id".to_string(), Value::Int(1)),
                        ("name".to_string(), Value::String("test".to_string())),
                        (
                            "list".to_string(),
                            Value::List(vec![Value::Int(1), Value::Int(2)]),
                        ),
                        (
                            "obj".to_string(),
                            Value::Object(
                                vec![("key".to_string(), Value::String("value".to_string()))]
                                    .into_iter()
                                    .collect(),
                            ),
                        ),
                    ]
                    .into(),
                ),
                skip_if: None,
                include_if: None,
                omit_from_response: false,
            })],
        };

        insta::assert_snapshot!(
          selection_set,
          @r#"{field1(id: 1, list: [1, 2], name: "test", obj: {key: "value"})}"#
        )
    }

    #[test]
    fn merge_selection_set_keeps_fields_with_different_conditions_separate() {
        let mut target = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "reviews".to_string(),
                selections: SelectionSet::default(),
                alias: None,
                arguments: None,
                skip_if: None,
                include_if: Some("first".to_string()),
                omit_from_response: false,
            })],
        };
        let source = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "reviews".to_string(),
                selections: SelectionSet::default(),
                alias: None,
                arguments: None,
                skip_if: None,
                include_if: Some("second".to_string()),
                omit_from_response: false,
            })],
        };

        merge_selection_set(&mut target, &source, false).unwrap();

        assert_eq!(target.items.len(), 2);
    }

    #[test]
    fn merge_selection_set_merges_fields_with_same_condition() {
        let mut target = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "reviews".to_string(),
                selections: SelectionSet {
                    items: vec![SelectionItem::Field(FieldSelection {
                        name: "id".to_string(),
                        selections: SelectionSet::default(),
                        alias: None,
                        arguments: None,
                        skip_if: None,
                        include_if: None,
                        omit_from_response: false,
                    })],
                },
                alias: None,
                arguments: None,
                skip_if: None,
                include_if: Some("cond".to_string()),
                omit_from_response: false,
            })],
        };
        let source = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection {
                name: "reviews".to_string(),
                selections: SelectionSet {
                    items: vec![SelectionItem::Field(FieldSelection {
                        name: "body".to_string(),
                        selections: SelectionSet::default(),
                        alias: None,
                        arguments: None,
                        skip_if: None,
                        include_if: None,
                        omit_from_response: false,
                    })],
                },
                alias: None,
                arguments: None,
                skip_if: None,
                include_if: Some("cond".to_string()),
                omit_from_response: false,
            })],
        };

        merge_selection_set(&mut target, &source, false).unwrap();

        assert_eq!(target.items.len(), 1);

        let SelectionItem::Field(field) = &target.items[0] else {
            panic!("expected field selection");
        };

        assert_eq!(field.selections.items.len(), 2);
    }

    #[test]
    fn merge_selection_set_keeps_inline_fragments_with_different_conditions_separate() {
        let mut target = SelectionSet {
            items: vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                type_condition: "User".to_string(),
                selections: SelectionSet::default(),
                skip_if: None,
                include_if: Some("first".to_string()),
            })],
        };
        let source = SelectionSet {
            items: vec![SelectionItem::InlineFragment(InlineFragmentSelection {
                type_condition: "User".to_string(),
                selections: SelectionSet::default(),
                skip_if: None,
                include_if: Some("second".to_string()),
            })],
        };

        merge_selection_set(&mut target, &source, false).unwrap();

        assert_eq!(target.items.len(), 2);
    }

    #[test]
    fn merge_selection_set_refuses_two_fields_under_one_key() {
        let parse = |query: &str| -> SelectionSet {
            match crate::query_planner::utils::parsing::parse_operation(query)
                .definitions
                .first()
            {
                Some(query_ast::Definition::Operation(
                    query_ast::OperationDefinition::SelectionSet(s),
                )) => s.clone().into(),
                _ => panic!("expected a selection set"),
            }
        };
        let mut target = parse(r#"{ me { price(currency: "GBP") } }"#);

        // Other arguments, or another field, under `price` on the same object.
        for source in [
            r#"{ me { price(currency: "EUR") } }"#,
            "{ me { price: id } }",
        ] {
            let error = merge_selection_set(&mut target.clone(), &parse(source), false);
            assert_eq!(error.unwrap_err().0, "price");
        }
        // The same value, and a different key, still merge.
        merge_selection_set(
            &mut target,
            &parse(r#"{ me { price(currency: "GBP") } }"#),
            false,
        )
        .unwrap();
        merge_selection_set(
            &mut target,
            &parse(r#"{ me { eur: price(currency: "EUR") } }"#),
            false,
        )
        .unwrap();
        assert_eq!(
            target.to_string(),
            r#"{me{price(currency: "GBP") eur: price(currency: "EUR")}}"#
        );
    }

    #[test]
    fn merge_selection_set_prefers_visible_typename_over_skipped() {
        let mut target = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection::new_skipped_typename())],
        };
        let source = SelectionSet {
            items: vec![SelectionItem::Field(FieldSelection::new_typename())],
        };

        merge_selection_set(&mut target, &source, false).unwrap();

        let [SelectionItem::Field(field)] = target.items.as_slice() else {
            panic!("expected exactly one __typename field");
        };

        assert_eq!(field.name, "__typename");
        assert!(!field.omit_from_response);
    }

    #[test]
    // Field Condition should only be equal to Condition::SkipAndInclude
    fn field_condition_skip_and_include() {
        let skip_cond = Some(Condition::Skip("skip".to_string()));
        let include_cond: Option<Condition> = Some(Condition::Include("include".to_string()));
        let skip_and_include_cond = Some(Condition::SkipAndInclude {
            skip: "skip".to_string(),
            include: "include".to_string(),
        });
        let field = FieldSelection {
            name: "name".to_string(),
            selections: SelectionSet::default(),
            alias: None,
            arguments: None,
            skip_if: Some("skip".to_string()),
            include_if: Some("include".to_string()),
            omit_from_response: false,
        };
        assert!(!field_condition_equal(&skip_cond, &field));
        assert!(!field_condition_equal(&include_cond, &field));
        assert!(field_condition_equal(&skip_and_include_cond, &field));
    }

    #[test]
    // Fragment Condition should only be equal to Condition::SkipAndInclude
    fn fragment_condition_skip_and_include() {
        let skip_cond = Some(Condition::Skip("skip".to_string()));
        let include_cond: Option<Condition> = Some(Condition::Include("include".to_string()));
        let skip_and_include_cond = Some(Condition::SkipAndInclude {
            skip: "skip".to_string(),
            include: "include".to_string(),
        });
        let fragment = InlineFragmentSelection {
            type_condition: "Product".to_string(),
            selections: SelectionSet::default(),
            skip_if: Some("skip".to_string()),
            include_if: Some("include".to_string()),
        };
        assert!(!fragment_condition_equal(&skip_cond, &fragment));
        assert!(!fragment_condition_equal(&include_cond, &fragment));
        assert!(fragment_condition_equal(&skip_and_include_cond, &fragment));
    }

    fn parse(query: &str) -> SelectionSet {
        use graphql_tools::parser::query::{Definition, OperationDefinition};

        match crate::query_planner::utils::parsing::parse_operation(query)
            .definitions
            .first()
        {
            Some(Definition::Operation(OperationDefinition::SelectionSet(s))) => s.clone().into(),
            _ => panic!("expected a selection set"),
        }
    }

    /// Another argument, alias or condition is another value, and the same field under the
    /// same type condition is found inside the fragment.
    #[test]
    fn contains_only_the_same_values() {
        let fetched =
            parse(r#"{ price(currency: "GBP") a @include(if: $x) ... on Cat { whiskers } }"#);

        assert!(fetched.contains(&parse(r#"{ price(currency: "GBP") }"#)));
        assert!(!fetched.contains(&parse(r#"{ price(currency: "EUR") }"#)));
        assert!(!fetched.contains(&parse(r#"{ gbp: price(currency: "GBP") }"#)));
        assert!(fetched.contains(&parse("{ a @include(if: $x) }")));
        assert!(!fetched.contains(&parse("{ a @include(if: $y) }")));
        assert!(!fetched.contains(&parse("{ a }")));
        assert!(fetched.contains(&parse("{ ... on Cat { whiskers } }")));
        assert!(!fetched.contains(&parse("{ ... on Dog { whiskers } }")));
    }
}
