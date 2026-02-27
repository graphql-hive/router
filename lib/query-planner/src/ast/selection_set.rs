use graphql_tools::parser::query as query_ast;
use serde::{ser::SerializeSeq, Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt::{Debug, Display},
    hash::Hash,
};

use crate::{
    ast::merge_path::{Condition, MergePath, Segment},
    utils::pretty_display::{get_indent, PrettyDisplay},
};

use super::{arguments::ArgumentsMap, selection_item::SelectionItem};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SelectionSet {
    pub items: Vec<SelectionItem>,
}

impl<'a, T: query_ast::Text<'a>> From<query_ast::SelectionSet<'a, T>> for SelectionSet {
    fn from(selection_set: query_ast::SelectionSet<'a, T>) -> Self {
        Self {
            items: selection_set
                .items
                .into_iter()
                .map(|item| item.into())
                .collect::<Vec<SelectionItem>>(),
        }
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
    pub fn with_new_selections(&self, selections: SelectionSet) -> Self {
        FieldSelection {
            name: self.name.clone(),
            alias: self.alias.clone(),
            selections,
            arguments: self.arguments.clone(),
            skip_if: self.skip_if.clone(),
            include_if: self.include_if.clone(),
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

fn selection_item_is_subset_of(source: &SelectionItem, target: &SelectionItem) -> bool {
    match (source, target) {
        (SelectionItem::Field(source_field), SelectionItem::Field(target_field)) => {
            if source_field.name != target_field.name {
                return false;
            }

            if source_field.is_leaf() != target_field.is_leaf() {
                return false;
            }

            selection_items_are_subset_of(
                &source_field.selections.items,
                &target_field.selections.items,
            )
        }
        // TODO: support fragments
        _ => false,
    }
}

pub fn merge_selection_set(target: &mut SelectionSet, source: &SelectionSet, as_first: bool) {
    if source.items.is_empty() {
        return;
    }

    let mut pending_items = Vec::with_capacity(source.items.len());
    for source_item in source.items.iter() {
        let mut found = false;
        for target_item in target.items.iter_mut() {
            match (source_item, target_item) {
                (SelectionItem::Field(source_field), SelectionItem::Field(target_field)) => {
                    if source_field == target_field {
                        found = true;
                        merge_selection_set(
                            &mut target_field.selections,
                            &source_field.selections,
                            as_first,
                        );
                        break;
                    }
                }
                (
                    SelectionItem::InlineFragment(source_fragment),
                    SelectionItem::InlineFragment(target_fragment),
                ) => {
                    if source_fragment.type_condition == target_fragment.type_condition {
                        found = true;
                        merge_selection_set(
                            &mut target_fragment.selections,
                            &source_fragment.selections,
                            as_first,
                        );
                        break;
                    }
                }
                _ => {}
            }
        }

        if !found {
            pending_items.push(source_item.clone())
        }
    }

    if !pending_items.is_empty() {
        if as_first {
            let mut new_items = pending_items;
            new_items.append(&mut target.items);
            target.items = new_items;
        } else {
            target.items.extend(pending_items);
        }
    }
}

pub fn find_selection_set_by_path_mut<'a>(
    root_selection_set: &'a mut SelectionSet,
    path: &MergePath,
) -> Option<&'a mut SelectionSet> {
    let mut current_selection_set = root_selection_set;

    for path_element in path.inner.iter() {
        match path_element {
            Segment::List => {
                continue;
            }
            Segment::TypeCondition(type_names, condition) => {
                let next_selection_set_option =
                    current_selection_set
                        .items
                        .iter_mut()
                        .find_map(|item| match item {
                            SelectionItem::Field(_) => None,
                            SelectionItem::InlineFragment(f) => {
                                if type_names.contains(&f.type_condition)
                                    && fragment_condition_equal(condition, f)
                                {
                                    Some(&mut f.selections)
                                } else {
                                    None
                                }
                            }
                            SelectionItem::FragmentSpread(_) => None,
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
            Segment::Field(field_name, args_hash, condition) => {
                let next_selection_set_option =
                    current_selection_set
                        .items
                        .iter_mut()
                        .find_map(|item| match item {
                            SelectionItem::Field(field) => {
                                if field.selection_identifier() == field_name
                                    && field.arguments_hash() == *args_hash
                                    && field_condition_equal(condition, field)
                                {
                                    Some(&mut field.selections)
                                } else {
                                    None
                                }
                            }
                            SelectionItem::InlineFragment(..) => None,
                            SelectionItem::FragmentSpread(_) => None,
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
        }
    }
    Some(current_selection_set)
}

pub fn field_condition_equal(cond: &Option<Condition>, field: &FieldSelection) -> bool {
    match cond {
        Some(cond) => match cond {
            Condition::Include(var_name) => {
                field.include_if.as_ref().is_some_and(|v| v == var_name)
            }
            Condition::Skip(var_name) => field.skip_if.as_ref().is_some_and(|v| v == var_name),
        },
        None => field.include_if.is_none() && field.skip_if.is_none(),
    }
}

fn fragment_condition_equal(cond: &Option<Condition>, fragment: &InlineFragmentSelection) -> bool {
    match cond {
        Some(cond) => match cond {
            Condition::Include(var_name) => {
                fragment.include_if.as_ref().is_some_and(|v| v == var_name)
            }
            Condition::Skip(var_name) => fragment.skip_if.as_ref().is_some_and(|v| v == var_name),
        },
        None => fragment.include_if.is_none() && fragment.skip_if.is_none(),
    }
}

/// Find the arguments conflicts between two selections.
/// Returns a vector of tuples containing the indices of conflicting fields in both "source" and "other"
/// Both indices are returned in order to allow for easy resolution of conflicts later, in either side.
pub fn find_arguments_conflicts(
    source: &SelectionSet,
    other: &SelectionSet,
) -> Vec<(usize, usize)> {
    other
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, other_selection)| {
            if let SelectionItem::Field(other_field) = other_selection {
                let other_identifier = other_field.selection_identifier();
                let other_args_hash = other_field.arguments_hash();

                let existing_in_self =
                    source
                        .items
                        .iter()
                        .enumerate()
                        .find_map(|(self_index, self_selection)| {
                            if let SelectionItem::Field(self_field) = self_selection {
                                // If the field selection identifier matches and the arguments hash is different,
                                // then it means that we can't merge the two input siblings
                                if self_field.selection_identifier() == other_identifier
                                    && self_field.arguments_hash() != other_args_hash
                                {
                                    return Some(self_index);
                                }
                            }

                            None
                        });

                if let Some(existing_index) = existing_in_self {
                    return Some((existing_index, index));
                }

                return None;
            }

            None
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::ast::value::Value;

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
                        })],
                    },
                    alias: Some("f2".to_string()),
                    arguments: None,
                    skip_if: None,
                    include_if: None,
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
            })],
        };

        insta::assert_snapshot!(
          selection_set,
          @r#"{field1(id: 1, list: [1, 2], name: "test", obj: {key: "value"})}"#
        )
    }
}
