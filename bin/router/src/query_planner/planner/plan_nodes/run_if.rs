use std::fmt::{Display, Formatter, Result as FmtResult};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Variables that all have to say yes: every `include_if` is `true` and every `skip_if` is
/// `false`. An empty group always holds. The names are sorted and unique, and no name is in
/// both. Only `new` makes one, reading it from JSON goes through it too.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "ConditionGroupJson")]
pub struct ConditionGroup {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    include_if: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    skip_if: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConditionGroupJson {
    #[serde(default)]
    include_if: Vec<String>,
    #[serde(default)]
    skip_if: Vec<String>,
}

impl TryFrom<ConditionGroupJson> for ConditionGroup {
    type Error = &'static str;

    fn try_from(json: ConditionGroupJson) -> Result<Self, Self::Error> {
        Self::new(json.include_if, json.skip_if)
            .ok_or("a condition group can't both include and skip a variable")
    }
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

/// When a fetch, or a group of an alias's paths, runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RunIf {
    #[default]
    Always,
    /// Nothing it'd fetch is ever asked for, like `@include(if: $a) @skip(if: $a)` on every
    /// field.
    Never,
    /// When any of the groups holds. Only `any_of` makes one, and keeps it normalized: at least
    /// one group, no empty one, no group that holds whenever a smaller one does
    /// (`$a || ($a && $b)` is just `$a`), sorted and unique.
    AnyOf(Box<[ConditionGroup]>),
}

impl RunIf {
    pub fn any_of(groups: impl IntoIterator<Item = ConditionGroup>) -> Self {
        let mut groups: Vec<ConditionGroup> = groups.into_iter().collect();
        if groups.iter().any(ConditionGroup::is_empty) {
            return Self::Always;
        }
        if groups.is_empty() {
            return Self::Never;
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
                    return Self::Always;
                }
            }
        }
        kept.sort_unstable();
        Self::AnyOf(kept.into_boxed_slice())
    }

    /// When any of them holds, like a fetch runs when any of its fields is asked for.
    /// Nothing at all is never.
    pub fn or(run_ifs: impl IntoIterator<Item = Self>) -> Self {
        let mut groups = Vec::new();
        for run_if in run_ifs {
            match run_if {
                Self::Always => return Self::Always,
                Self::Never => {}
                Self::AnyOf(more) => groups.extend(more),
            }
        }
        Self::any_of(groups)
    }

    pub fn is_always(&self) -> bool {
        matches!(self, Self::Always)
    }

    pub fn holds(&self, value: impl Fn(&str) -> Option<bool>) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::AnyOf(groups) => groups.iter().any(|group| group.holds(&value)),
        }
    }

    /// Both of them.
    pub fn and(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Always, only) | (only, Self::Always) => only.clone(),
            (Self::Never, _) | (_, Self::Never) => Self::Never,
            (Self::AnyOf(left), Self::AnyOf(right)) => Self::any_of(
                left.iter()
                    .flat_map(|l| right.iter().filter_map(move |r| l.and(r))),
            ),
        }
    }

    /// What's left to check when `known` is already known to hold, like the conditions of the
    /// `Include`/`Skip` nodes around a fetch.
    pub fn assuming(&self, known: &ConditionGroup) -> Self {
        let Self::AnyOf(groups) = self else {
            return self.clone();
        };
        let known: Vec<(&str, bool)> = known.literals().collect();
        Self::any_of(groups.iter().filter_map(|group| {
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

/// In JSON `Always` is left out (`null` on its own), `Never` is no groups and `AnyOf` is its
/// groups. Reading it goes through `any_of`, so it comes back normalized.
impl Serialize for RunIf {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Always => serializer.serialize_none(),
            Self::Never => serializer.collect_seq(&[] as &[ConditionGroup]),
            Self::AnyOf(groups) => serializer.collect_seq(groups.iter()),
        }
    }
}

impl<'de> Deserialize<'de> for RunIf {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(
            match Option::<Vec<ConditionGroup>>::deserialize(deserializer)? {
                None => Self::Always,
                Some(groups) => Self::any_of(groups),
            },
        )
    }
}

impl Display for RunIf {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let groups = match self {
            Self::Always => return write!(f, "true"),
            Self::Never => return write!(f, "false"),
            Self::AnyOf(groups) => groups,
        };
        let parenthesize = groups.len() > 1;
        for (index, group) in groups.iter().enumerate() {
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

    fn run_if(groups: &[&[(&str, bool)]]) -> RunIf {
        RunIf::any_of(groups.iter().map(|literals| group(literals)))
    }

    fn shown(run_if: &RunIf) -> String {
        run_if.to_string()
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
        assert_eq!(shown(&run_if(&[&[("a", true)], &[]])), "true");
        // no groups is never
        assert_eq!(shown(&RunIf::any_of([])), "false");
        // `$a || !$a`
        assert_eq!(shown(&run_if(&[&[("a", true)], &[("a", false)]])), "true");
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
        assert_eq!(shown(&a.and(&b)), "$a && !$b");
        assert_eq!(shown(&a.and(&RunIf::Always)), "$a");
        assert_eq!(shown(&RunIf::Always.and(&RunIf::Always)), "true");

        let a_or_b = run_if(&[&[("a", true)], &[("b", true)]]);
        let c = run_if(&[&[("c", true)]]);
        assert_eq!(shown(&a_or_b.and(&c)), "($a && $c) || ($b && $c)");
        // `$a && !$a` never holds
        let not_a = run_if(&[&[("a", false)]]);
        assert_eq!(shown(&a.and(&not_a)), "false");
    }

    #[test]
    fn assuming_drops_what_is_known() {
        let either = run_if(&[&[("a", true), ("b", true)], &[("a", true)]]);
        // absorbed into `$a`, which is known
        assert_eq!(shown(&either.assuming(&group(&[("a", true)]))), "true");

        let either = run_if(&[&[("a", true), ("b", true)], &[("c", true)]]);
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
        let either = run_if(&[&[("a", true), ("b", false)], &[("c", true)]]);
        let values = |pairs: &'static [(&'static str, Option<bool>)]| {
            move |name: &str| pairs.iter().find(|(n, _)| *n == name).and_then(|(_, v)| *v)
        };
        assert!(either.holds(values(&[("a", Some(true)), ("b", Some(false))])));
        assert!(either.holds(values(&[("c", Some(true))])));
        assert!(!either.holds(values(&[("a", Some(true)), ("b", Some(true))])));
        // a missing variable fails its group, for `skip_if` too
        assert!(!either.holds(values(&[("a", Some(true))])));
        assert!(!RunIf::Never.holds(values(&[])));
    }

    /// Every group over `names`, each variable left out, `true` or `false`.
    fn all_groups(names: &[&'static str]) -> Vec<ConditionGroup> {
        let mut all: Vec<Vec<(&str, bool)>> = vec![vec![]];
        for name in names {
            all = all
                .into_iter()
                .flat_map(|literals| {
                    let with = |include| [literals.clone(), vec![(*name, include)]].concat();
                    [with(true), with(false), literals]
                })
                .collect();
        }
        all.iter().map(|literals| group(literals)).collect()
    }

    /// Every list of up to `size` of `groups`, duplicates included.
    fn all_lists(groups: &[ConditionGroup], size: usize) -> Vec<Vec<ConditionGroup>> {
        let mut lists = vec![vec![]];
        let mut last = lists.clone();
        for _ in 0..size {
            last = last
                .iter()
                .flat_map(|list| {
                    groups
                        .iter()
                        .map(|g| [list.clone(), vec![g.clone()]].concat())
                })
                .collect();
            lists.extend(last.iter().cloned());
        }
        lists
    }

    const NAMES: [&str; 3] = ["a", "b", "c"];

    /// Each bit of `values` is a variable of `NAMES`. Validation fills every variable in, so
    /// `true` and `false` is all a condition ever sees.
    fn value(values: u32) -> impl Fn(&str) -> Option<bool> {
        move |name| {
            let index = NAMES.iter().position(|n| *n == name).unwrap();
            Some(values >> index & 1 == 1)
        }
    }

    fn runs(run_if: &RunIf, values: u32) -> bool {
        run_if.holds(value(values))
    }

    fn all_values() -> std::ops::Range<u32> {
        0..1 << NAMES.len()
    }

    #[test]
    fn any_of_holds_when_one_of_its_groups_does() {
        for list in all_lists(&all_groups(&NAMES), 3) {
            let run_if = RunIf::any_of(list.clone());
            for values in all_values() {
                let expected = list.iter().any(|group| group.holds(value(values)));
                assert_eq!(runs(&run_if, values), expected, "{list:?} at {values:03b}");
            }
            // normalized already, nothing left to do
            if let RunIf::AnyOf(groups) = &run_if {
                assert_eq!(RunIf::any_of(groups.to_vec()), run_if);
            }
        }
    }

    #[test]
    fn and_holds_when_both_do() {
        let lists = all_lists(&all_groups(&NAMES[..2]), 2);
        for left in &lists {
            let left = RunIf::any_of(left.clone());
            for right in &lists {
                let right = RunIf::any_of(right.clone());
                let both = left.and(&right);
                for values in all_values() {
                    assert_eq!(
                        runs(&both, values),
                        runs(&left, values) && runs(&right, values),
                        "{left:?} and {right:?} at {values:03b}"
                    );
                }
            }
        }
    }

    #[test]
    fn assuming_holds_the_same_whenever_what_is_known_does() {
        let groups = all_groups(&NAMES);
        for list in all_lists(&groups, 2) {
            let run_if = RunIf::any_of(list);
            for known in &groups {
                let left = run_if.assuming(known);
                for values in all_values().filter(|values| known.holds(value(*values))) {
                    assert_eq!(
                        runs(&left, values),
                        run_if.holds(value(values)),
                        "{run_if:?} assuming {known:?} at {values:03b}"
                    );
                }
            }
        }
    }

    #[test]
    fn json_round_trip_holds_the_same() {
        for list in all_lists(&all_groups(&NAMES), 2) {
            let run_if = RunIf::any_of(list);
            let json = serde_json::to_string(&run_if).unwrap();
            let back: RunIf = serde_json::from_str(&json).unwrap();
            assert_eq!(back, run_if, "{json}");
            for values in all_values() {
                assert_eq!(back.holds(value(values)), run_if.holds(value(values)));
            }
        }
    }

    #[test]
    fn json_shape() {
        use crate::query_planner::planner::plan_nodes::PathGroup;
        use serde_json::json;

        let path_group = |json| serde_json::from_value::<PathGroup>(json).unwrap();
        let to_json = |group: &PathGroup| serde_json::to_value(group).unwrap();

        // no `runIf` is always
        let always = path_group(json!({ "paths": [] }));
        assert_eq!(always.run_if, RunIf::Always);
        assert!(always.is_active(|_| None));
        assert_eq!(to_json(&always), json!({ "paths": [] }));

        // no groups is never
        let never = path_group(json!({ "runIf": [], "paths": [] }));
        assert_eq!(never.run_if, RunIf::Never);
        assert!(!never.is_active(|_| Some(true)));
        assert_eq!(to_json(&never), json!({ "runIf": [], "paths": [] }));

        // a group lists only what it has
        let groups = json!({
            "runIf": [{ "skipIf": ["c"] }, { "includeIf": ["a"], "skipIf": ["b"] }],
            "paths": []
        });
        let some = path_group(groups.clone());
        assert_eq!(some.run_if.to_string(), "!$c || ($a && !$b)");
        assert_eq!(to_json(&some), groups);
    }

    #[test]
    fn a_group_read_from_json_is_normalized() {
        let read = |json| serde_json::from_value::<ConditionGroup>(json);

        let group = read(serde_json::json!({ "includeIf": ["b", "a", "b"], "skipIf": ["c", "c"] }))
            .unwrap();
        assert_eq!(group.to_string(), "$a && $b && !$c");
        assert_eq!(
            serde_json::to_value(&group).unwrap(),
            serde_json::json!({ "includeIf": ["a", "b"], "skipIf": ["c"] })
        );

        // it would never hold, and nothing we write looks like that
        let error = read(serde_json::json!({ "includeIf": ["a"], "skipIf": ["a"] })).unwrap_err();
        assert!(
            error.to_string().contains("both include and skip"),
            "{error}"
        );
    }

    #[test]
    fn groups_read_from_json_are_normalized() {
        let read = |json| serde_json::from_value::<RunIf>(json).unwrap().to_string();
        assert_eq!(
            read(serde_json::json!([
                { "includeIf": ["b"] },
                { "includeIf": ["a", "b"] },
                { "includeIf": ["a"] },
                { "includeIf": ["a"] }
            ])),
            "$a || $b"
        );
        // nothing we write looks like these, they still hold as their groups do
        assert_eq!(read(serde_json::json!([{}])), "true");
        assert_eq!(
            read(serde_json::json!([{ "includeIf": ["a"] }, { "skipIf": ["a"] }])),
            "true"
        );
        // a group that never holds isn't read at all, it can't turn into always
        assert!(serde_json::from_value::<RunIf>(serde_json::json!([
            { "includeIf": ["a"], "skipIf": ["a"] }
        ]))
        .is_err());
    }
}
