use std::fmt::{Display, Formatter, Result as FmtResult};

use serde::{Deserialize, Serialize};

/// Variables that all have to say yes: every `include_if` is `true` and every `skip_if` is
/// `false`. An empty group always holds. The names are sorted and unique.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionGroup {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_if: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip_if: Vec<String>,
}

impl ConditionGroup {
    /// `None` when it can never hold, a variable that has to be both `true` and `false`.
    pub fn new(
        include_if: impl IntoIterator<Item = String>,
        skip_if: impl IntoIterator<Item = String>,
    ) -> Option<Self> {
        let mut include_if: Vec<String> = include_if.into_iter().collect();
        let mut skip_if: Vec<String> = skip_if.into_iter().collect();
        include_if.sort_unstable();
        include_if.dedup();
        skip_if.sort_unstable();
        skip_if.dedup();
        if include_if
            .iter()
            .any(|name| skip_if.binary_search(name).is_ok())
        {
            return None;
        }
        Some(Self {
            include_if,
            skip_if,
        })
    }

    /// The variables, `true` for an `include_if` and `false` for a `skip_if`.
    pub fn literals(&self) -> impl Iterator<Item = (&str, bool)> {
        let include = self.include_if.iter().map(|name| (name.as_str(), true));
        let skip = self.skip_if.iter().map(|name| (name.as_str(), false));
        include.chain(skip)
    }

    pub fn from_literals<S: AsRef<str>>(
        literals: impl IntoIterator<Item = (S, bool)>,
    ) -> Option<Self> {
        let (include_if, skip_if): (Vec<_>, Vec<_>) =
            literals.into_iter().partition(|(_, include)| *include);
        Self::new(
            include_if
                .into_iter()
                .map(|(name, _)| name.as_ref().to_string()),
            skip_if
                .into_iter()
                .map(|(name, _)| name.as_ref().to_string()),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.include_if.is_empty() && self.skip_if.is_empty()
    }

    /// Both groups at once, `None` when they contradict each other.
    pub fn and(&self, other: &Self) -> Option<Self> {
        Self::new(
            self.include_if.iter().chain(&other.include_if).cloned(),
            self.skip_if.iter().chain(&other.skip_if).cloned(),
        )
    }

    /// Holds whenever `other` does, as every variable we ask for, `other` asks for too.
    fn is_implied_by(&self, other: &Self) -> bool {
        let contains = |all: &[String], some: &[String]| {
            some.iter().all(|name| all.binary_search(name).is_ok())
        };
        contains(&other.include_if, &self.include_if) && contains(&other.skip_if, &self.skip_if)
    }

    /// `value` says if a variable is `true` (`Some(true)`), anything else (`Some(false)`, `null`
    /// included) or missing (`None`). A missing variable fails the group, the same way a
    /// `Condition` node runs neither of its sides then.
    pub fn holds(&self, value: impl Fn(&str) -> Option<bool>) -> bool {
        self.literals()
            .all(|(name, include)| value(name) == Some(include))
    }
}

impl Display for ConditionGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        for (index, (name, include)) in self.literals().enumerate() {
            if index > 0 {
                write!(f, " && ")?;
            }
            let not = if include { "" } else { "!" };
            write!(f, "{not}${name}")?;
        }
        Ok(())
    }
}

/// Runs when any of the groups holds. Kept normalized: no group that never holds, no group that
/// holds whenever a smaller one does (`$a || ($a && $b)` is just `$a`), sorted and unique.
/// "Always" isn't a `RunIf`, everything that builds one returns `None` for it.
/// No groups at all means never.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunIf(Box<[ConditionGroup]>);

impl RunIf {
    /// `None` when it always holds.
    pub fn any_of(groups: impl IntoIterator<Item = ConditionGroup>) -> Option<Self> {
        let mut groups: Vec<ConditionGroup> = groups.into_iter().collect();
        if groups.iter().any(ConditionGroup::is_empty) {
            return None;
        }
        groups.sort_unstable();
        groups.dedup();
        // drop every group a smaller one already covers
        let mut kept: Vec<ConditionGroup> = Vec::with_capacity(groups.len());
        groups.sort_by_key(|group| group.include_if.len() + group.skip_if.len());
        for group in groups {
            if !kept.iter().any(|smaller| smaller.is_implied_by(&group)) {
                kept.push(group);
            }
        }
        // `$a || !$a` is always. Only the plain one-variable case, no general tautology check.
        for group in &kept {
            if let [(name, include)] = group.literals().collect::<Vec<_>>()[..] {
                let opposite = ConditionGroup::from_literals([(name, !include)]);
                if opposite.is_some_and(|opposite| kept.contains(&opposite)) {
                    return None;
                }
            }
        }
        kept.sort_unstable();
        Some(Self(kept.into_boxed_slice()))
    }

    pub fn groups(&self) -> &[ConditionGroup] {
        &self.0
    }

    pub fn holds(&self, value: impl Fn(&str) -> Option<bool>) -> bool {
        self.0.iter().any(|group| group.holds(&value))
    }

    /// Both of them, `None` (always) gives the other one.
    pub fn and(left: Option<&Self>, right: Option<&Self>) -> Option<Self> {
        match (left, right) {
            (None, None) => None,
            (Some(only), None) | (None, Some(only)) => Some(only.clone()),
            (Some(left), Some(right)) => Self::any_of(
                left.0
                    .iter()
                    .flat_map(|l| right.0.iter().filter_map(move |r| l.and(r))),
            ),
        }
    }

    /// What's left to check when `known` is already known to hold, like the conditions of the
    /// `Include`/`Skip` nodes around a fetch. `None` when nothing is left.
    pub fn assuming(&self, known: &ConditionGroup) -> Option<Self> {
        let known: Vec<(&str, bool)> = known.literals().collect();
        Self::any_of(self.0.iter().filter_map(|group| {
            let mut left = Vec::new();
            for (name, include) in group.literals() {
                match known.iter().find(|(known_name, _)| *known_name == name) {
                    Some((_, known_include)) if *known_include == include => {}
                    // it asks for the opposite of what we know, so it can't hold
                    Some(_) => return None,
                    None => left.push((name, include)),
                }
            }
            ConditionGroup::from_literals(left)
        }))
    }
}

impl Display for RunIf {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        if self.0.is_empty() {
            return write!(f, "false");
        }
        let parenthesize = self.0.len() > 1;
        for (index, group) in self.0.iter().enumerate() {
            if index > 0 {
                write!(f, " || ")?;
            }
            if parenthesize && group.literals().nth(1).is_some() {
                write!(f, "({group})")?;
            } else {
                write!(f, "{group}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(literals: &[(&str, bool)]) -> ConditionGroup {
        ConditionGroup::from_literals(literals.iter().copied()).expect("not a contradiction")
    }

    fn run_if(groups: &[&[(&str, bool)]]) -> Option<RunIf> {
        RunIf::any_of(groups.iter().map(|literals| group(literals)))
    }

    fn shown(run_if: &Option<RunIf>) -> String {
        run_if
            .as_ref()
            .map_or("always".to_string(), ToString::to_string)
    }

    #[test]
    fn groups_are_sorted_unique_and_drop_contradictions() {
        let sorted = ConditionGroup::new(
            ["b".to_string(), "a".to_string(), "b".to_string()],
            ["c".to_string()],
        )
        .unwrap();
        assert_eq!(sorted.to_string(), "$a && $b && !$c");
        assert_eq!(
            ConditionGroup::new(["a".to_string()], ["a".to_string()]),
            None
        );
    }

    #[test]
    fn any_of_normalizes() {
        // absorption
        assert_eq!(
            shown(&run_if(&[&[("a", true)], &[("a", true), ("b", true)]])),
            "$a"
        );
        // duplicates and order
        assert_eq!(
            shown(&run_if(&[&[("b", true)], &[("a", false)], &[("b", true)]])),
            "!$a || $b"
        );
        // an empty group is always
        assert_eq!(shown(&run_if(&[&[("a", true)], &[]])), "always");
        // no groups is never
        assert_eq!(shown(&RunIf::any_of([])), "false");
        // `$a || !$a`
        assert_eq!(shown(&run_if(&[&[("a", true)], &[("a", false)]])), "always");
        // not a general tautology check
        assert_eq!(
            shown(&run_if(&[&[("a", true), ("b", true)], &[("a", false)]])),
            "!$a || ($a && $b)"
        );
    }

    #[test]
    fn and() {
        let a = run_if(&[&[("a", true)]]);
        let b = run_if(&[&[("b", false)]]);
        assert_eq!(shown(&RunIf::and(a.as_ref(), b.as_ref())), "$a && !$b");
        assert_eq!(shown(&RunIf::and(a.as_ref(), None)), "$a");
        assert_eq!(shown(&RunIf::and(None, None)), "always");

        let a_or_b = run_if(&[&[("a", true)], &[("b", true)]]);
        let c = run_if(&[&[("c", true)]]);
        assert_eq!(
            shown(&RunIf::and(a_or_b.as_ref(), c.as_ref())),
            "($a && $c) || ($b && $c)"
        );
        // `$a && !$a` never holds
        let not_a = run_if(&[&[("a", false)]]);
        assert_eq!(shown(&RunIf::and(a.as_ref(), not_a.as_ref())), "false");
    }

    #[test]
    fn assuming_drops_what_is_known() {
        let either = run_if(&[&[("a", true), ("b", true)], &[("a", true)]]).unwrap();
        // absorbed into `$a`, which is known
        assert_eq!(shown(&either.assuming(&group(&[("a", true)]))), "always");

        let either = run_if(&[&[("a", true), ("b", true)], &[("c", true)]]).unwrap();
        assert_eq!(shown(&either.assuming(&group(&[("a", true)]))), "$b || $c");
        // a group asking for the opposite can't hold
        assert_eq!(shown(&either.assuming(&group(&[("a", false)]))), "$c");
        assert_eq!(
            shown(&either.assuming(&group(&[("a", false), ("c", false)]))),
            "false"
        );
    }

    #[test]
    fn holds_needs_every_variable_of_one_group() {
        let either = run_if(&[&[("a", true), ("b", false)], &[("c", true)]]).unwrap();
        let values = |pairs: &'static [(&'static str, Option<bool>)]| {
            move |name: &str| pairs.iter().find(|(n, _)| *n == name).and_then(|(_, v)| *v)
        };
        assert!(either.holds(values(&[("a", Some(true)), ("b", Some(false))])));
        assert!(either.holds(values(&[("c", Some(true))])));
        assert!(!either.holds(values(&[("a", Some(true)), ("b", Some(true))])));
        // a missing variable fails its group, for `skip_if` too
        assert!(!either.holds(values(&[("a", Some(true))])));
        assert!(!RunIf::any_of([]).unwrap().holds(values(&[])));
    }
}
