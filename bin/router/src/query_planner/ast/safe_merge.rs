use std::collections::{BTreeSet, HashMap};

use tracing::trace;

use crate::query_planner::{
    ast::{
        merge_path::{FieldPathSegment, MergePath, Segment},
        selection_item::SelectionItem,
        selection_set::{FieldSelection, SelectionSet},
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphState},
};

#[derive(Debug, Clone)]
pub struct SafeSelectionSetMerger<'a> {
    aliases_counter: u64,
    supergraph: &'a SupergraphState,
}

pub enum ConflictsLookupResult {
    Merged,
    Conflict(ConflictResolutionLocation),
    Copy,
}

pub enum MergeAction {
    Copy(SelectionItem),
    Conflict(ConflictResolutionLocation),
}

#[derive(Debug)]
pub enum ConflictResolutionLocation {
    Source { source_item_idx: usize },
    Target { target_item_idx: usize },
}

pub type AliasesRecords = Vec<(MergePath, String)>;

const ALIAS_PREFIX: &str = "_internal_qp_alias_";

/// Two plain fields with the same response key and other arguments or alias. Neither side is
/// there for a `@requires`, so both are what the client asked for, and there's nothing to alias.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Field '{0}' has a conflict that can't be resolved with an alias")]
pub struct UnresolvableConflict(pub String);

impl<'a> SafeSelectionSetMerger<'a> {
    pub fn new(supergraph: &'a SupergraphState) -> Self {
        Self {
            aliases_counter: 0,
            supergraph,
        }
    }

    /// Moves the counter past every internal alias in `set`. A step's selections end up in one
    /// operation, so a name used anywhere in it, even in another fragment, can't be picked
    /// again. `safe_next_alias_name` only sees the items next to the new field.
    pub fn skip_aliases_in(&mut self, set: &SelectionSet) {
        for item in &set.items {
            match item {
                SelectionItem::Field(field) => {
                    // Inputs are patched the other way around, `price: _internal_qp_alias_0`.
                    for name in field.alias.iter().chain([&field.name]) {
                        if let Some(number) = name
                            .strip_prefix(ALIAS_PREFIX)
                            .and_then(|number| number.parse::<u64>().ok())
                        {
                            self.aliases_counter = self.aliases_counter.max(number + 1);
                        }
                    }
                    self.skip_aliases_in(&field.selections);
                }
                SelectionItem::InlineFragment(fragment) => {
                    self.skip_aliases_in(&fragment.selections)
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    pub fn safe_next_alias_name(&mut self, target_existing: &[SelectionItem]) -> String {
        loop {
            let alias = format!("{ALIAS_PREFIX}{}", self.aliases_counter);
            self.aliases_counter += 1;

            let exists = target_existing
                .iter()
                .find(|v| matches!(v, SelectionItem::Field(f) if f.name == alias || f.alias.as_ref().is_some_and(|v| v == &alias)));

            if exists.is_none() {
                return alias;
            }
        }
    }

    pub fn merge_selection_set(
        &mut self,
        target: &mut SelectionSet,
        source: &SelectionSet,
        (self_used_for_requires, other_used_for_requires): (bool, bool),
        as_first: bool,
        surroundings: Option<&SelectionSet>,
    ) -> Result<AliasesRecords, UnresolvableConflict> {
        let mut aliases_performed: AliasesRecords = Vec::new();
        self.merge_selection_set_inner(
            target,
            source,
            (self_used_for_requires, other_used_for_requires),
            as_first,
            MergePath::default(),
            surroundings,
            &mut aliases_performed,
        )?;

        // The merge above only compares items at the same level, with the same conditions.
        // Fields that land in different fragments, or have different `@include`/`@skip`,
        // are copied next to each other, so check the whole result.
        self.check_conflicts(target)?;

        Ok(aliases_performed)
    }

    /// Fails when `selection_set` has fields that can't be in one operation together, see
    /// `find_field_conflict`.
    pub fn check_conflicts(
        &self,
        selection_set: &SelectionSet,
    ) -> Result<(), UnresolvableConflict> {
        match find_field_conflict(self.supergraph, &[(selection_set, None)]) {
            Some(field_name) => Err(UnresolvableConflict(field_name)),
            None => Ok(()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn merge_selection_set_inner(
        &mut self,
        target: &mut SelectionSet,
        source: &SelectionSet,
        (self_used_for_requires, other_used_for_requires): (bool, bool),
        as_first: bool,
        response_path: MergePath,
        // Fields next to `target` that end up on the same objects, only known at the top.
        surroundings: Option<&SelectionSet>,
        aliases_performed: &mut AliasesRecords,
    ) -> Result<(), UnresolvableConflict> {
        if source.items.is_empty() {
            return Ok(());
        }

        // A vector to store pending merge/conflict resolution actions
        let mut pending_items: Vec<MergeAction> = Vec::with_capacity(source.items.len());

        for (source_item_idx, source_item) in source.items.iter().enumerate() {
            // We assume we add the new field, unless we find a conflict or the field already exists and then we can merge
            let mut decision = ConflictsLookupResult::Copy;

            for (target_item_idx, target_item) in target.items.iter_mut().enumerate() {
                match (source_item, target_item) {
                    (SelectionItem::Field(source_field), SelectionItem::Field(target_field))
                        if source_field.selection_identifier()
                            == target_field.selection_identifier()
                            && source_field.include_if == target_field.include_if
                            && source_field.skip_if == target_field.skip_if =>
                    {
                        let has_conflict = source_field.arguments_hash()
                            != target_field.arguments_hash()
                            || source_field.alias != target_field.alias;

                        if !has_conflict {
                            trace!(
                                "found a matching field {}, will proceed with merging",
                                source_field.name,
                            );
                            decision = ConflictsLookupResult::Merged;

                            let next_path = response_path.push(Segment::Field(
                                FieldPathSegment::new(
                                    source_field.name.clone(),
                                    source_field.alias.clone(),
                                ),
                                source_field.arguments_hash(),
                                source_field.into(),
                            ));

                            self.merge_selection_set_inner(
                                &mut target_field.selections,
                                &source_field.selections,
                                (self_used_for_requires, other_used_for_requires),
                                as_first,
                                next_path,
                                None,
                                aliases_performed,
                            )?;

                            break;
                        } else {
                            let conflict = match (self_used_for_requires, other_used_for_requires) {
                                (true, false) => {
                                    ConflictResolutionLocation::Target { target_item_idx }
                                }
                                (false, true) | (true, true) => {
                                    ConflictResolutionLocation::Source { source_item_idx }
                                }
                                (false, false) => {
                                    return Err(UnresolvableConflict(source_field.name.clone()))
                                }
                            };

                            trace!(
                                    "found a conflicting field '{}' ({} != {}), will resolve the conflict on the {:?} side",
                                    source_field.name,
                                    source_field.arguments_hash(),
                                    target_field.arguments_hash(),
                                    conflict
                                );

                            decision = ConflictsLookupResult::Conflict(conflict);
                        }
                    }
                    (
                        SelectionItem::InlineFragment(source_fragment),
                        SelectionItem::InlineFragment(target_fragment),
                    ) if source_fragment.type_condition == target_fragment.type_condition
                        && source_fragment.include_if == target_fragment.include_if
                        && source_fragment.skip_if == target_fragment.skip_if =>
                    {
                        decision = ConflictsLookupResult::Merged;

                        let next_path = response_path.push(Segment::TypeCondition(
                            BTreeSet::from([source_fragment.type_condition.clone()]),
                            source_fragment.into(),
                        ));

                        self.merge_selection_set_inner(
                            &mut target_fragment.selections,
                            &source_fragment.selections,
                            (self_used_for_requires, other_used_for_requires),
                            as_first,
                            next_path,
                            None,
                            aliases_performed,
                        )?;
                        break;
                    }
                    _ => {}
                }
            }

            match decision {
                // If fields were merged, nothing to do here.
                ConflictsLookupResult::Merged => {}
                // If there's a conflict, we should register it to resolution
                // A decision to solve the conflict on the "target" side means that we'll alias the existing field, and copy the other field.
                // A decision to solve the conflict on the "source" means that we'll copy the source field and then alias it.
                ConflictsLookupResult::Conflict(conflict) => {
                    if let ConflictResolutionLocation::Target { .. } = conflict {
                        pending_items.push(MergeAction::Copy(source_item.clone()));
                    }

                    pending_items.push(MergeAction::Conflict(conflict));
                }
                // In case the field does not exists, and doesn't have a conflict, we can just copy it as-is.
                ConflictsLookupResult::Copy => {
                    let mut copied = SelectionSet {
                        items: vec![source_item.clone()],
                    };
                    // The loop above only compares items with the same conditions. What we
                    // copy can still clash with a field under another `@include` or in a
                    // fragment, it all ends up on the same objects. The side that's there for
                    // a `@requires` gets an alias.
                    match (self_used_for_requires, other_used_for_requires) {
                        (_, true) => self.alias_clashes(
                            &mut copied,
                            &[&*target]
                                .into_iter()
                                .chain(surroundings)
                                .collect::<Vec<_>>(),
                            &response_path,
                            aliases_performed,
                        ),
                        // We can't alias what's around `target`, if that clashes too, the
                        // conflict check below fails the merge.
                        (true, false) => self.alias_clashes(
                            target,
                            &[&copied],
                            &response_path,
                            aliases_performed,
                        ),
                        (false, false) => {}
                    }
                    pending_items.extend(copied.items.into_iter().map(MergeAction::Copy));
                }
            }
        }

        for pending_item in pending_items {
            match pending_item {
                // In case of copy, just add the field as-is.
                MergeAction::Copy(item) => {
                    if as_first {
                        target.items.insert(0, item);
                    } else {
                        target.items.push(item);
                    }
                }
                // In case of conflict, we need to resolve it by aliasing in one of the sides of the conflict.
                MergeAction::Conflict(conflict_resolution_location) => {
                    let next_alias = self.safe_next_alias_name(&target.items);

                    match conflict_resolution_location {
                        ConflictResolutionLocation::Source { source_item_idx } => {
                            if let Some(SelectionItem::Field(field_selection)) =
                                source.items.get(source_item_idx)
                            {
                                let mut new_field = field_selection.clone();
                                new_field.alias = Some(next_alias.clone());

                                let pair = (
                                    response_path.push(Segment::Field(
                                        FieldPathSegment::named(new_field.name.clone()),
                                        new_field.arguments_hash(),
                                        (&new_field).into(),
                                    )),
                                    next_alias,
                                );

                                aliases_performed.push(pair);

                                target.items.push(SelectionItem::Field(new_field));
                            }
                        }
                        ConflictResolutionLocation::Target { target_item_idx } => {
                            if let Some(SelectionItem::Field(field_selection)) =
                                target.items.get_mut(target_item_idx)
                            {
                                field_selection.alias = Some(next_alias.clone());

                                let pair = (
                                    response_path.push(Segment::Field(
                                        FieldPathSegment::named(field_selection.name.clone()),
                                        field_selection.arguments_hash(),
                                        field_selection.into(),
                                    )),
                                    next_alias,
                                );
                                aliases_performed.push(pair);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Aliases the fields of `set` that have the same response key as a field of `others` on
    /// the same objects, but aren't the same field. All are looked through inline fragments.
    fn alias_clashes(
        &mut self,
        set: &mut SelectionSet,
        others: &[&SelectionSet],
        response_path: &MergePath,
        aliases_performed: &mut AliasesRecords,
    ) {
        let mut other_fields = Vec::new();
        for other in others {
            collect_fields(other, None, &mut other_fields);
        }
        let mut set_fields = Vec::new();
        collect_fields(set, None, &mut set_fields);
        let mut taken: BTreeSet<String> = other_fields
            .iter()
            .chain(&set_fields)
            .map(|(field, _)| field.selection_identifier().to_string())
            .collect();

        let supergraph = self.supergraph;
        let counter = &mut self.aliases_counter;
        for_each_field_mut(set, None, response_path, &mut |field, type_name, path| {
            let clashes = other_fields.iter().any(|(other, other_type)| {
                other.selection_identifier() == field.selection_identifier()
                    && (other.name != field.name
                        || other.arguments_hash() != field.arguments_hash())
                    && can_share_objects(supergraph, type_name, *other_type)
            });
            if !clashes {
                return;
            }
            let alias = loop {
                let alias = format!("{ALIAS_PREFIX}{counter}");
                *counter += 1;
                if taken.insert(alias.clone()) {
                    break alias;
                }
            };
            trace!(
                "aliasing '{}' as '{alias}', it clashes with another field",
                field.name
            );
            field.alias = Some(alias.clone());
            let location = path.push(Segment::Field(
                FieldPathSegment::named(field.name.clone()),
                field.arguments_hash(),
                (&*field).into(),
            ));
            aliases_performed.push((location, alias));
        });
    }
}

/// Calls `f` for every field of `set` at its own level, looking through inline fragments,
/// with the type it's selected on and the path to where it sits.
fn for_each_field_mut(
    set: &mut SelectionSet,
    type_name: Option<&str>,
    path: &MergePath,
    f: &mut impl FnMut(&mut FieldSelection, Option<&str>, &MergePath),
) {
    for item in set.items.iter_mut() {
        match item {
            SelectionItem::Field(field) => f(field, type_name, path),
            SelectionItem::InlineFragment(fragment) => {
                let path = path.push(Segment::TypeCondition(
                    BTreeSet::from([fragment.type_condition.clone()]),
                    (&*fragment).into(),
                ));
                for_each_field_mut(
                    &mut fragment.selections,
                    Some(&fragment.type_condition),
                    &path,
                    f,
                );
            }
            SelectionItem::FragmentSpread(_) => {}
        }
    }
}

/// Fields under two different object types can never be on the same object.
fn can_share_objects(supergraph: &SupergraphState, a: Option<&str>, b: Option<&str>) -> bool {
    let is_object = |name: &str| {
        matches!(
            supergraph.definitions.get(name),
            Some(SupergraphDefinition::Object(_))
        )
    };
    match (a, b) {
        (Some(a), Some(b)) => a == b || !is_object(a) || !is_object(b),
        _ => true,
    }
}

/// GraphQL wants fields with the same response key in a selection set to be the same field,
/// with the same arguments, including the ones inside inline fragments. `@include`/`@skip`
/// don't change that, even when they can never be true together. Returns the name of the
/// first field that breaks it.
///
/// `sets` are selection sets that end up as one, like the sub-selections of fields that share
/// a response key. Each comes with the type it's selected on, if we know it.
///
/// Only fields under two different object types are left alone, they can never be on the same
/// object. Anything else, like an interface and one of its objects, is checked.
fn find_field_conflict(
    supergraph: &SupergraphState,
    sets: &[(&SelectionSet, Option<&str>)],
) -> Option<String> {
    let mut fields: Vec<(&FieldSelection, Option<&str>)> = Vec::new();
    for (set, type_name) in sets {
        collect_fields(set, *type_name, &mut fields);
    }

    let mut by_response_key: HashMap<&str, Vec<(&FieldSelection, Option<&str>)>> = HashMap::new();
    for field in fields {
        by_response_key
            .entry(field.0.selection_identifier())
            .or_default()
            .push(field);
    }

    for group in by_response_key.values() {
        for (i, (a, a_type)) in group.iter().enumerate() {
            let mut children = vec![(&a.selections, None)];
            for (b, b_type) in &group[i + 1..] {
                if !can_share_objects(supergraph, *a_type, *b_type) {
                    continue;
                }
                if a.name != b.name || a.arguments_hash() != b.arguments_hash() {
                    return Some(a.name.clone());
                }
                children.push((&b.selections, None));
            }
            if let Some(field_name) = find_field_conflict(supergraph, &children) {
                return Some(field_name);
            }
        }
    }

    None
}

/// Fields of `set`, and of the inline fragments in it, with the type they're selected on.
fn collect_fields<'a>(
    set: &'a SelectionSet,
    type_name: Option<&'a str>,
    fields: &mut Vec<(&'a FieldSelection, Option<&'a str>)>,
) {
    for item in &set.items {
        match item {
            SelectionItem::Field(field) => fields.push((field, type_name)),
            SelectionItem::InlineFragment(fragment) => {
                collect_fields(&fragment.selections, Some(&fragment.type_condition), fields)
            }
            SelectionItem::FragmentSpread(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::query::{Definition, OperationDefinition};

    use lazy_static::lazy_static;

    use crate::query_planner::{
        ast::{safe_merge::SafeSelectionSetMerger, selection_set::SelectionSet},
        state::supergraph_state::SupergraphState,
        utils::parsing::{parse_operation, parse_schema},
    };

    lazy_static! {
        static ref SUPERGRAPH: SupergraphState = SupergraphState::new(&parse_schema(
            r#"
            directive @join__type(graph: join__Graph!, key: join__FieldSet) repeatable on OBJECT | INTERFACE
            scalar join__FieldSet
            enum join__Graph { A @join__graph(name: "a", url: "") }
            interface Node @join__type(graph: A) { id: ID! }
            interface Pet @join__type(graph: A) { id: ID! }
            type Cat implements Node & Pet @join__type(graph: A) { id: ID! }
            type Dog implements Node & Pet @join__type(graph: A) { id: ID! }
            type Photo @join__type(graph: A) { id: ID! }
            type Query @join__type(graph: A) { node: Node }
            "#,
        ));
    }

    fn new_merger() -> SafeSelectionSetMerger<'static> {
        SafeSelectionSetMerger::new(&SUPERGRAPH)
    }

    fn parse_selection_set(input: &str) -> SelectionSet {
        let op = parse_operation(input);

        match op.definitions.first() {
            Some(Definition::Operation(OperationDefinition::SelectionSet(s))) => s.clone().into(),
            _ => panic!("bad input"),
        }
    }

    #[test]
    fn valid_selection_no_conflicts() {
        let mut a = parse_selection_set("{ a }");
        let b = parse_selection_set("{ b }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (true, false), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{a b}");
    }

    #[test]
    fn conflict_without_requires_is_an_error() {
        let mut a = parse_selection_set("{ a { b(x: 1) } }");
        let b = parse_selection_set("{ a { b(x: 2) } }");

        let mut merger = new_merger();
        assert!(merger
            .merge_selection_set(&mut a, &b, (false, false), false, None)
            .is_err());
    }

    #[test]
    fn conflict_under_different_conditions_is_an_error() {
        let merge = |a: &str, b: &str| {
            let mut a = parse_selection_set(a);
            new_merger().merge_selection_set(
                &mut a,
                &parse_selection_set(b),
                (false, false),
                false,
                None,
            )
        };

        // In fragments with different conditions.
        assert!(merge(
            "{ ... on Photo @include(if: $a) { t(w: 1) } }",
            "{ ... on Photo @include(if: $b) { t(w: 2) } }",
        )
        .is_err());
        // Plain fields with different conditions.
        assert!(merge("{ t(w: 1) @include(if: $a) }", "{ t(w: 2) @skip(if: $a) }").is_err());
        // Deeper, under fields that don't merge because of their conditions.
        assert!(merge(
            "{ p @include(if: $a) { t(w: 1) } }",
            "{ p @include(if: $b) { t(w: 2) } }",
        )
        .is_err());
        // Different type conditions can have different arguments.
        assert!(merge("{ ... on Cat { t(w: 1) } }", "{ ... on Dog { t(w: 2) } }").is_ok());
        // An interface and an object that can be one: some `Node`s are `Cat`s.
        assert!(merge("{ ... on Node { t(w: 1) } }", "{ ... on Cat { t(w: 2) } }").is_err());
        // Two interfaces can share objects.
        assert!(merge("{ ... on Node { t(w: 1) } }", "{ ... on Pet { t(w: 2) } }").is_err());
        // A type we don't know about gets checked too.
        assert!(merge("{ ... on Cat { t(w: 1) } }", "{ ... on Nope { t(w: 2) } }").is_err());
        // Same arguments are fine.
        assert!(merge(
            "{ ... on Photo @include(if: $a) { t(w: 1) } }",
            "{ ... on Photo @include(if: $b) { t(w: 1) } }",
        )
        .is_ok());
    }

    /// Fields under other conditions or in fragments aren't compared by the merge itself,
    /// but they still clash, so the side that's there for a `@requires` gets an alias.
    #[test]
    fn conflict_under_different_conditions_gets_an_alias() {
        let merge = |a: &str, b: &str, sides, surroundings: Option<&str>| {
            let mut a = parse_selection_set(a);
            let surroundings = surroundings.map(parse_selection_set);
            let aliases = new_merger()
                .merge_selection_set(
                    &mut a,
                    &parse_selection_set(b),
                    sides,
                    false,
                    surroundings.as_ref(),
                )
                .unwrap();
            let aliases: Vec<_> = aliases
                .iter()
                .map(|(path, alias)| format!("{path} as {alias}"))
                .collect();
            format!("{a} {aliases:?}")
        };

        insta::assert_snapshot!(
            merge("{ t(w: 1) @include(if: $a) }", "{ t(w: 2) }", (false, true), None),
            @r#"{t(w: 1) @include(if: $a) _internal_qp_alias_0: t(w: 2)} ["t as _internal_qp_alias_0"]"#
        );
        insta::assert_snapshot!(
            merge("{ t(w: 1) }", "{ ... on Photo @include(if: $a) { t(w: 2) } }", (false, true), None),
            @r#"{t(w: 1) ...on Photo @include(if: $a){_internal_qp_alias_0: t(w: 2)}} ["|[Photo] @include(if: $a).t as _internal_qp_alias_0"]"#
        );
        insta::assert_snapshot!(
            merge("{ ... on Photo @include(if: $a) { t(w: 1) } }", "{ t(w: 2) }", (true, false), None),
            @r#"{...on Photo @include(if: $a){_internal_qp_alias_0: t(w: 1)} t(w: 2)} ["|[Photo] @include(if: $a).t as _internal_qp_alias_0"]"#
        );
        // What's around the merge target counts too, and so do the aliases in it.
        insta::assert_snapshot!(
            merge(
                "{ id }",
                "{ t(w: 2) }",
                (false, true),
                Some("{ ... on Photo { id t(w: 1) _internal_qp_alias_0: t(w: 3) } }"),
            ),
            @r#"{id _internal_qp_alias_1: t(w: 2)} ["t as _internal_qp_alias_1"]"#
        );
    }

    #[test]
    fn mix_field_name_and_alias() {
        let mut a = parse_selection_set("{ a }");
        let b = parse_selection_set("{ a: b }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (true, false), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{_internal_qp_alias_0: a a: b}");
    }

    #[test]
    fn simple_merge_with_same_field() {
        let mut a = parse_selection_set("{ a }");
        let b = parse_selection_set("{ a }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (true, false), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{a}");
    }

    #[test]
    fn simple_merge_args_conflict() {
        let mut a = parse_selection_set("{ a(i: 1) }");
        let b = parse_selection_set("{ a(i: 2) }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (false, true), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{a(i: 1) _internal_qp_alias_0: a(i: 2)}");
    }

    #[test]
    fn inherent_conflict() {
        let mut a = parse_selection_set("{ a(i: 1) _internal_qp_alias_0 }");
        let b = parse_selection_set("{ a(i: 2) }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (true, true), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{a(i: 1) _internal_qp_alias_0 _internal_qp_alias_1: a(i: 2)}");
    }

    #[test]
    fn inherent_conflict_alias() {
        let mut a = parse_selection_set("{ a(i: 1) _internal_qp_alias_0: test }");
        let b = parse_selection_set("{ a(i: 2) }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (false, true), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{a(i: 1) _internal_qp_alias_0: test _internal_qp_alias_1: a(i: 2)}");
    }

    #[test]
    fn multiple_simple_conflicts() {
        let mut a = parse_selection_set("{ a(i: 1) }");
        let b = parse_selection_set("{ a(i: 2) }");
        let c = parse_selection_set("{ a(i: 3) }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (false, true), false, None)
            .unwrap();
        merger
            .merge_selection_set(&mut a, &c, (false, true), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{a(i: 1) _internal_qp_alias_0: a(i: 2) _internal_qp_alias_1: a(i: 3)}");
    }

    #[test]
    fn nested_conflict() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");
        let c = parse_selection_set("{ p { a(i: 3) } }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (false, true), false, None)
            .unwrap();
        merger
            .merge_selection_set(&mut a, &c, (false, true), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{p{a(i: 1) _internal_qp_alias_0: a(i: 2) _internal_qp_alias_1: a(i: 3)}}");
    }

    #[test]
    fn multiple_merge_processes() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");
        let c = parse_selection_set("{ p { a(i: 3) } }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (false, true), false, None)
            .unwrap();
        let mut merger2 = new_merger();
        merger2
            .merge_selection_set(&mut a, &c, (false, true), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{p{a(i: 1) _internal_qp_alias_0: a(i: 2) _internal_qp_alias_1: a(i: 3)}}");
    }

    #[test]
    fn preferred_side_source() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (false, true), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{p{a(i: 1) _internal_qp_alias_0: a(i: 2)}}");
    }

    #[test]
    fn preferred_side_target() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (true, false), false, None)
            .unwrap();

        insta::assert_snapshot!(a, @"{p{_internal_qp_alias_0: a(i: 1) a(i: 2)}}");
    }

    #[test]
    fn merge_path_nested() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");

        let mut merger = new_merger();
        let merge_locations = merger
            .merge_selection_set(&mut a, &b, (false, true), false, None)
            .unwrap();
        assert_eq!(merge_locations.len(), 1);
        insta::assert_snapshot!(merge_locations[0].0, @"p.a");
        insta::assert_snapshot!(merge_locations[0].1, @"_internal_qp_alias_0");
    }
}
