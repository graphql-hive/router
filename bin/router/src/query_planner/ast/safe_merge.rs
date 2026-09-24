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

    pub fn safe_next_alias_name(&mut self, target_existing: &[SelectionItem]) -> String {
        loop {
            let alias = format!("_internal_qp_alias_{}", self.aliases_counter);
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
    ) -> Result<AliasesRecords, UnresolvableConflict> {
        let mut aliases_performed: AliasesRecords = Vec::new();
        self.merge_selection_set_inner(
            target,
            source,
            (self_used_for_requires, other_used_for_requires),
            as_first,
            MergePath::default(),
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

    pub fn merge_selection_set_inner(
        &mut self,
        target: &mut SelectionSet,
        source: &SelectionSet,
        (self_used_for_requires, other_used_for_requires): (bool, bool),
        as_first: bool,
        response_path: MergePath,
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
                    pending_items.push(MergeAction::Copy(source_item.clone()));
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
        let is_object = |name: &str| {
            matches!(
                supergraph.definitions.get(name),
                Some(SupergraphDefinition::Object(_))
            )
        };
        let same_scope = |a: Option<&str>, b: Option<&str>| match (a, b) {
            (Some(a), Some(b)) => a == b || !is_object(a) || !is_object(b),
            _ => true,
        };

        for (i, (a, a_type)) in group.iter().enumerate() {
            let mut children = vec![(&a.selections, None)];
            for (b, b_type) in &group[i + 1..] {
                if !same_scope(*a_type, *b_type) {
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
            .merge_selection_set(&mut a, &b, (true, false), false)
            .unwrap();

        insta::assert_snapshot!(a, @"{a b}");
    }

    #[test]
    fn conflict_without_requires_is_an_error() {
        let mut a = parse_selection_set("{ a { b(x: 1) } }");
        let b = parse_selection_set("{ a { b(x: 2) } }");

        let mut merger = new_merger();
        assert!(merger
            .merge_selection_set(&mut a, &b, (false, false), false)
            .is_err());
    }

    #[test]
    fn conflict_under_different_conditions_is_an_error() {
        let merge = |a: &str, b: &str| {
            let mut a = parse_selection_set(a);
            new_merger().merge_selection_set(&mut a, &parse_selection_set(b), (true, true), false)
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

    #[test]
    fn mix_field_name_and_alias() {
        let mut a = parse_selection_set("{ a }");
        let b = parse_selection_set("{ a: b }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (true, false), false)
            .unwrap();

        insta::assert_snapshot!(a, @"{_internal_qp_alias_0: a a: b}");
    }

    #[test]
    fn simple_merge_with_same_field() {
        let mut a = parse_selection_set("{ a }");
        let b = parse_selection_set("{ a }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (true, false), false)
            .unwrap();

        insta::assert_snapshot!(a, @"{a}");
    }

    #[test]
    fn simple_merge_args_conflict() {
        let mut a = parse_selection_set("{ a(i: 1) }");
        let b = parse_selection_set("{ a(i: 2) }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (false, true), false)
            .unwrap();

        insta::assert_snapshot!(a, @"{a(i: 1) _internal_qp_alias_0: a(i: 2)}");
    }

    #[test]
    fn inherent_conflict() {
        let mut a = parse_selection_set("{ a(i: 1) _internal_qp_alias_0 }");
        let b = parse_selection_set("{ a(i: 2) }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (true, true), false)
            .unwrap();

        insta::assert_snapshot!(a, @"{a(i: 1) _internal_qp_alias_0 _internal_qp_alias_1: a(i: 2)}");
    }

    #[test]
    fn inherent_conflict_alias() {
        let mut a = parse_selection_set("{ a(i: 1) _internal_qp_alias_0: test }");
        let b = parse_selection_set("{ a(i: 2) }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (false, true), false)
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
            .merge_selection_set(&mut a, &b, (false, true), false)
            .unwrap();
        merger
            .merge_selection_set(&mut a, &c, (false, true), false)
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
            .merge_selection_set(&mut a, &b, (false, true), false)
            .unwrap();
        merger
            .merge_selection_set(&mut a, &c, (false, true), false)
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
            .merge_selection_set(&mut a, &b, (false, true), false)
            .unwrap();
        let mut merger2 = new_merger();
        merger2
            .merge_selection_set(&mut a, &c, (false, true), false)
            .unwrap();

        insta::assert_snapshot!(a, @"{p{a(i: 1) _internal_qp_alias_0: a(i: 2) _internal_qp_alias_1: a(i: 3)}}");
    }

    #[test]
    fn preferred_side_source() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (false, true), false)
            .unwrap();

        insta::assert_snapshot!(a, @"{p{a(i: 1) _internal_qp_alias_0: a(i: 2)}}");
    }

    #[test]
    fn preferred_side_target() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");

        let mut merger = new_merger();
        merger
            .merge_selection_set(&mut a, &b, (true, false), false)
            .unwrap();

        insta::assert_snapshot!(a, @"{p{_internal_qp_alias_0: a(i: 1) a(i: 2)}}");
    }

    #[test]
    fn merge_path_nested() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");

        let mut merger = new_merger();
        let merge_locations = merger
            .merge_selection_set(&mut a, &b, (false, true), false)
            .unwrap();
        assert_eq!(merge_locations.len(), 1);
        insta::assert_snapshot!(merge_locations[0].0, @"p.a");
        insta::assert_snapshot!(merge_locations[0].1, @"_internal_qp_alias_0");
    }
}
