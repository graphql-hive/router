//! When a fetch runs, from the `@include`/`@skip` on its fields.

use std::fmt::{Display, Formatter, Result as FmtResult};

use serde::{Deserialize, Serialize};

use crate::query_planner::ast::{selection_item::SelectionItem, selection_set::SelectionSet};

/// Variables that all have to say yes: every `include_if` is `true` and no `skip_if` is. An
/// empty group always holds. `new` keeps the names sorted, and never one in both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionGroup {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_if: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip_if: Vec<String>,
}

impl ConditionGroup {
    /// `None` when a variable would have to be both `true` and `false`.
    pub fn new(mut include_if: Vec<String>, mut skip_if: Vec<String>) -> Option<Self> {
        for names in [&mut include_if, &mut skip_if] {
            names.sort_unstable();
            names.dedup();
        }
        if include_if.iter().any(|name| skip_if.contains(name)) {
            return None;
        }
        Some(Self {
            include_if,
            skip_if,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.include_if.is_empty() && self.skip_if.is_empty()
    }

    pub fn holds(&self, is_true: impl Fn(&str) -> bool) -> bool {
        self.include_if.iter().all(|name| is_true(name))
            && !self.skip_if.iter().any(|name| is_true(name))
    }

    /// Both at once.
    pub fn and(&self, other: &Self) -> Option<Self> {
        Self::new(
            self.include_if
                .iter()
                .chain(&other.include_if)
                .cloned()
                .collect(),
            self.skip_if.iter().chain(&other.skip_if).cloned().collect(),
        )
    }

    /// What's left to check once `known` holds, `None` when it can't hold then.
    pub fn assuming(&self, known: &Self) -> Option<Self> {
        if self
            .include_if
            .iter()
            .any(|name| known.skip_if.contains(name))
            || self
                .skip_if
                .iter()
                .any(|name| known.include_if.contains(name))
        {
            return None;
        }
        let left = |names: &[String], known: &[String]| {
            names
                .iter()
                .filter(|name| !known.contains(name))
                .cloned()
                .collect()
        };
        Some(Self {
            include_if: left(&self.include_if, &known.include_if),
            skip_if: left(&self.skip_if, &known.skip_if),
        })
    }

    /// Holds whenever `other` does, everything we ask for `other` asks for too.
    fn is_implied_by(&self, other: &Self) -> bool {
        self.include_if
            .iter()
            .all(|name| other.include_if.contains(name))
            && self.skip_if.iter().all(|name| other.skip_if.contains(name))
    }
}

impl Display for ConditionGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let names = self
            .include_if
            .iter()
            .map(|name| format!("${name}"))
            .chain(self.skip_if.iter().map(|name| format!("!${name}")));
        write!(f, "{}", names.collect::<Vec<_>>().join(" && "))
    }
}

/// When any of `groups` holds: `None` when that's always, an empty list for never. Leaves out a
/// group a smaller one already covers, `$a || ($a && $b)` is `$a`.
pub fn any_of(mut groups: Vec<ConditionGroup>) -> Option<Vec<ConditionGroup>> {
    if groups.iter().any(ConditionGroup::is_empty) {
        return None;
    }
    groups.sort_by_key(|group| group.include_if.len() + group.skip_if.len());
    let mut kept: Vec<ConditionGroup> = Vec::with_capacity(groups.len());
    for group in groups {
        if !kept.iter().any(|smaller| smaller.is_implied_by(&group)) {
            kept.push(group);
        }
    }
    kept.sort_unstable();
    Some(kept)
}

/// When any field of `selection_sets` is asked for, what's left of it once `known` holds.
/// `None` is always, like `any_of`.
pub fn run_if_of_output<'a>(
    selection_sets: impl IntoIterator<Item = &'a SelectionSet>,
    known: &ConditionGroup,
) -> Option<Vec<ConditionGroup>> {
    let mut groups = Vec::new();
    for selection_set in selection_sets {
        groups.extend(groups_of_selection_set(selection_set));
    }
    any_of(
        groups
            .iter()
            .filter_map(|group| group.assuming(known))
            .collect(),
    )
}

/// When any item is asked for, as groups. A field with no condition makes it always (an empty
/// group), so most selection sets stop at their first field.
fn groups_of_selection_set(selection_set: &SelectionSet) -> Vec<ConditionGroup> {
    let mut groups = Vec::new();
    for item in &selection_set.items {
        let (include_if, skip_if, inner) = match item {
            SelectionItem::Field(field) => (&field.include_if, &field.skip_if, None),
            SelectionItem::InlineFragment(fragment) => (
                &fragment.include_if,
                &fragment.skip_if,
                Some(&fragment.selections),
            ),
            SelectionItem::FragmentSpread(_) => return vec![ConditionGroup::default()],
        };
        // `@include(if: $a) @skip(if: $a)` is never asked for
        let Some(own) = ConditionGroup::new(
            include_if.iter().cloned().collect(),
            skip_if.iter().cloned().collect(),
        ) else {
            continue;
        };
        match inner {
            None => groups.push(own),
            Some(inner) => groups.extend(
                groups_of_selection_set(inner)
                    .iter()
                    .filter_map(|group| own.and(group)),
            ),
        }
        if groups.last().is_some_and(ConditionGroup::is_empty) {
            return vec![ConditionGroup::default()];
        }
    }
    groups
}

/// `$a || ($b && !$c)`, `false` for never.
pub fn display_run_if(groups: &[ConditionGroup]) -> String {
    if groups.is_empty() {
        return "false".to_string();
    }
    let parenthesize = groups.len() > 1;
    groups
        .iter()
        .map(
            |group| match parenthesize && group.include_if.len() + group.skip_if.len() > 1 {
                true => format!("({group})"),
                false => group.to_string(),
            },
        )
        .collect::<Vec<_>>()
        .join(" || ")
}

#[cfg(test)]
mod tests {
    use super::{any_of, run_if_of_output, ConditionGroup};
    use crate::query_planner::ast::selection_set::SelectionSet;
    use crate::query_planner::utils::parsing::parse_operation;

    fn group(include_if: &[&str], skip_if: &[&str]) -> ConditionGroup {
        let names = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
        ConditionGroup::new(names(include_if), names(skip_if)).unwrap()
    }

    fn selection_set(query: &str) -> SelectionSet {
        let document = parse_operation(query);
        let graphql_tools::parser::query::Definition::Operation(
            graphql_tools::parser::query::OperationDefinition::SelectionSet(selection_set),
        ) = document.definitions.into_iter().next().unwrap()
        else {
            panic!("a selection set");
        };
        selection_set.into()
    }

    #[test]
    fn a_smaller_group_covers_a_bigger_one() {
        let a = group(&["a"], &[]);
        assert_eq!(
            any_of(vec![group(&["a", "b"], &[]), a.clone()]),
            Some(vec![a])
        );
        assert_eq!(
            any_of(vec![group(&["a"], &[]), ConditionGroup::default()]),
            None
        );
        assert_eq!(any_of(Vec::new()), Some(Vec::new()));
    }

    #[test]
    fn a_fetch_runs_when_any_of_its_fields_is_asked_for() {
        let output = selection_set(
            "{ ... on T @include(if: $a) { x } ... on T @skip(if: $b) { ... on T @include(if: $c) { y } } }",
        );
        let none = ConditionGroup::default();
        assert_eq!(
            run_if_of_output([&output], &none),
            Some(vec![group(&["a"], &[]), group(&["c"], &["b"])])
        );
        // under `Include(if: $a)` the first one always runs
        assert_eq!(run_if_of_output([&output], &group(&["a"], &[])), None);
        // under `Skip(if: $a)` only the second one can
        assert_eq!(
            run_if_of_output([&output], &group(&[], &["a"])),
            Some(vec![group(&["c"], &["b"])])
        );
        // a field with no condition always runs
        assert_eq!(
            run_if_of_output(
                [&selection_set("{ x ... on T @include(if: $a) { y } }")],
                &none
            ),
            None
        );
        // `@include(if: $a) @skip(if: $a)` never does
        assert_eq!(
            run_if_of_output(
                [&selection_set("{ x @include(if: $a) @skip(if: $a) }")],
                &none
            ),
            Some(Vec::new())
        );
    }
}
