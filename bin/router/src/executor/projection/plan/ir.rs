use crate::executor::introspection::schema::FieldNullability;
use ahash::HashSet as AHashSet;
use bumpalo::collections::Vec as BumpVec;
use bumpalo::Bump;
use std::cmp::Ordering;
use std::ops::Deref;

/// A sorted, deduplicated set of type or enum names stored in the arena
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TypeSet<'a>(&'a [&'a str]);

impl<'a> Deref for TypeSet<'a> {
    type Target = [&'a str];

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a> TypeSet<'a> {
    pub(super) fn sorted(mut names: BumpVec<'a, &'a str>) -> Self {
        names.sort_unstable();
        names.dedup();
        Self(names.into_bump_slice())
    }

    pub(super) fn exact(arena: &'a Bump, name: &'a str) -> Self {
        Self(arena.alloc_slice_copy(&[name]))
    }

    pub(super) fn from_schema(arena: &'a Bump, values: &'a AHashSet<String>) -> Self {
        let mut names = BumpVec::with_capacity_in(values.len(), arena);
        names.extend(values.iter().map(String::as_str));
        Self::sorted(names)
    }

    pub(super) fn matches(self, type_name: &str) -> bool {
        self.binary_search(&type_name).is_ok()
    }

    pub(super) fn is_subset_of(self, other: Self) -> bool {
        self.iter().all(|name| other.matches(name))
    }

    pub(super) fn union(self, other: Self, arena: &'a Bump) -> Self {
        if self == other {
            return self;
        }
        let mut merged = BumpVec::with_capacity_in(self.len() + other.len(), arena);
        let (mut i, mut j) = (0, 0);
        while i < self.len() && j < other.len() {
            match self[i].cmp(other[j]) {
                Ordering::Less => {
                    merged.push(self[i]);
                    i += 1;
                }
                Ordering::Greater => {
                    merged.push(other[j]);
                    j += 1;
                }
                Ordering::Equal => {
                    merged.push(self[i]);
                    i += 1;
                    j += 1;
                }
            }
        }
        merged.extend_from_slice(&self[i..]);
        merged.extend_from_slice(&other[j..]);
        Self(merged.into_bump_slice())
    }

    pub(super) fn intersect(self, other: Self, arena: &'a Bump) -> Self {
        if self == other {
            return self;
        }

        let mut merged = BumpVec::with_capacity_in(self.len().min(other.len()), arena);

        let (mut i, mut j) = (0, 0);
        while i < self.len() && j < other.len() {
            match self[i].cmp(other[j]) {
                Ordering::Less => i += 1,
                Ordering::Greater => j += 1,
                Ordering::Equal => {
                    merged.push(self[i]);
                    i += 1;
                    j += 1;
                }
            }
        }

        Self(merged.into_bump_slice())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Condition<'a> {
    Always,
    IncludeIf(&'a str),
    SkipIf(&'a str),
    ParentType(TypeSet<'a>),
    FieldType(TypeSet<'a>),
    EnumValues(TypeSet<'a>),
    Or(&'a Condition<'a>, &'a Condition<'a>),
    And(&'a Condition<'a>, &'a Condition<'a>),
}

impl<'a> Condition<'a> {
    pub(super) fn and(self, right: Self, arena: &'a Bump) -> Self {
        use Condition::*;
        match (self, right) {
            (Always, other) | (other, Always) => other,
            (ParentType(l), ParentType(r)) => ParentType(l.intersect(r, arena)),
            (FieldType(l), FieldType(r)) => FieldType(l.intersect(r, arena)),
            (EnumValues(l), EnumValues(r)) => EnumValues(l.intersect(r, arena)),
            (left, right) => And(arena.alloc(left), arena.alloc(right)),
        }
    }

    pub(super) fn or(self, right: Self, arena: &'a Bump) -> Self {
        use Condition::*;
        if self == right {
            return self;
        }
        match (self, right) {
            (Always, _) | (_, Always) => Always,
            (ParentType(l), ParentType(r)) => ParentType(l.union(r, arena)),
            (FieldType(l), FieldType(r)) => FieldType(l.union(r, arena)),
            (EnumValues(l), EnumValues(r)) => EnumValues(l.union(r, arena)),
            (left, right) => Or(arena.alloc(left), arena.alloc(right)),
        }
    }

    pub(super) fn with_directives(
        mut self,
        include_if: Option<&'a str>,
        skip_if: Option<&'a str>,
        arena: &'a Bump,
    ) -> Self {
        if let Some(variable) = include_if {
            self = self.and(Self::IncludeIf(variable), arena);
        }
        if let Some(variable) = skip_if {
            self = self.and(Self::SkipIf(variable), arena);
        }
        self
    }

    fn map_leafs(self, arena: &'a Bump, f: &impl Fn(Self) -> Self) -> Self {
        match self {
            Self::And(left, right) => left
                .map_leafs(arena, f)
                .and(right.map_leafs(arena, f), arena),
            Self::Or(left, right) => left
                .map_leafs(arena, f)
                .or(right.map_leafs(arena, f), arena),
            leaf => f(leaf),
        }
    }

    /// Keeps only the parts of a parent condition that still apply to child selections.
    /// After we move into a field value, checks for the old parent object no longer fit.
    /// Only `@include` and `@skip` checks still make sense.
    pub(super) fn inherited_by_child(self, arena: &'a Bump) -> Self {
        self.map_leafs(arena, &|leaf| match leaf {
            Self::IncludeIf(_) | Self::SkipIf(_) => leaf,
            _ => Self::Always,
        })
    }

    /// Drops parent type checks already implied by the field's parent scope.
    ///
    /// The scope is checked by a guard before the condition even runs.
    /// So if `t` covers the whole scope, `ParentType(t)` can never fail.
    pub(super) fn without_redundant_parent_scope(
        self,
        scope: &Option<TypeSet<'a>>,
        arena: &'a Bump,
    ) -> Self {
        let Some(scope) = *scope else {
            return self;
        };
        self.map_leafs(arena, &|leaf| match leaf {
            Self::ParentType(t) if scope.is_subset_of(t) => Self::Always,
            leaf => leaf,
        })
    }
}

/// Schema-aware projection IR produced by collection and transformed by merging.
#[derive(Clone)]
pub(super) struct Field<'a> {
    pub(super) field_name: &'a str,
    pub(super) response_key: &'a str,
    pub(super) is_typename: bool,
    pub(super) nullability: &'a FieldNullability,
    pub(super) parent_scope: Option<TypeSet<'a>>,
    pub(super) condition: Condition<'a>,
    pub(super) value: Value<'a>,
}

#[derive(Clone)]
pub(super) enum Value<'a> {
    Passthrough,
    Children(BumpVec<'a, Field<'a>>),
}
