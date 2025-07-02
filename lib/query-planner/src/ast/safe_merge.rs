use tracing::trace;

use crate::ast::{
    merge_path::{MergePath, Segment},
    selection_item::SelectionItem,
    selection_set::SelectionSet,
};

#[derive(Debug, Clone, Default)]
pub struct SafeSelectionSetMerger {
    aliases_counter: u64,
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

impl SafeSelectionSetMerger {
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
    ) -> AliasesRecords {
        let mut aliases_performed: AliasesRecords = Vec::new();
        self.merge_selection_set_inner(
            target,
            source,
            (self_used_for_requires, other_used_for_requires),
            as_first,
            MergePath::default(),
            &mut aliases_performed,
        );

        aliases_performed
    }

    pub fn merge_selection_set_inner(
        &mut self,
        target: &mut SelectionSet,
        source: &SelectionSet,
        (self_used_for_requires, other_used_for_requires): (bool, bool),
        as_first: bool,
        response_path: MergePath,
        aliases_performed: &mut AliasesRecords,
    ) {
        if source.items.is_empty() {
            return;
        }

        // A vector to store pending merge/conflict resolution actions
        let mut pending_items: Vec<MergeAction> = Vec::with_capacity(source.items.len());

        for (source_item_idx, source_item) in source.items.iter().enumerate() {
            // We assume we add the new field, unless we find a conflict or the field already exists and then we can merge
            let mut decision = ConflictsLookupResult::Copy;

            for (target_item_idx, target_item) in target.items.iter_mut().enumerate() {
                match (source_item, target_item) {
                    (SelectionItem::Field(source_field), SelectionItem::Field(target_field)) => {
                        if source_field.selection_identifier()
                            == target_field.selection_identifier()
                            && source_field.include_if == target_field.include_if
                            && source_field.skip_if == target_field.skip_if
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
                                    source_field.name.clone(),
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
                                );

                                break;
                            } else {
                                let conflict =
                                    match (self_used_for_requires, other_used_for_requires) {
                                        (true, false) => {
                                            ConflictResolutionLocation::Target { target_item_idx }
                                        }
                                        (false, true) | (true, true) => {
                                            ConflictResolutionLocation::Source { source_item_idx }
                                        }
                                        (false, false) => panic!("Unexpected conflict"),
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
                    }
                    (
                        SelectionItem::InlineFragment(source_fragment),
                        SelectionItem::InlineFragment(target_fragment),
                    ) => {
                        if source_fragment.type_condition == target_fragment.type_condition
                            && source_fragment.include_if == target_fragment.include_if
                            && source_fragment.skip_if == target_fragment.skip_if
                        {
                            decision = ConflictsLookupResult::Merged;

                            let next_path = response_path.push(Segment::Cast(
                                source_fragment.type_condition.clone(),
                                source_fragment.into(),
                            ));

                            self.merge_selection_set_inner(
                                &mut target_fragment.selections,
                                &source_fragment.selections,
                                (self_used_for_requires, other_used_for_requires),
                                as_first,
                                next_path,
                                aliases_performed,
                            );
                            break;
                        }
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
                                        new_field.name.clone(),
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
                                        field_selection.name.clone(),
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
    }
}

#[cfg(test)]
mod tests {
    use graphql_parser::query::{Definition, OperationDefinition};

    use crate::{
        ast::{safe_merge::SafeSelectionSetMerger, selection_set::SelectionSet},
        utils::parsing::parse_operation,
    };

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

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (true, false), false);

        insta::assert_snapshot!(a, @"{a b}");
    }

    #[test]
    fn mix_field_name_and_alias() {
        let mut a = parse_selection_set("{ a }");
        let b = parse_selection_set("{ a: b }");

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (true, false), false);

        insta::assert_snapshot!(a, @"{_internal_qp_alias_0: a a: b}");
    }

    #[test]
    fn simple_merge_with_same_field() {
        let mut a = parse_selection_set("{ a }");
        let b = parse_selection_set("{ a }");

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (true, false), false);

        insta::assert_snapshot!(a, @"{a}");
    }

    #[test]
    fn simple_merge_args_conflict() {
        let mut a = parse_selection_set("{ a(i: 1) }");
        let b = parse_selection_set("{ a(i: 2) }");

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (false, true), false);

        insta::assert_snapshot!(a, @"{a(i: 1) _internal_qp_alias_0: a(i: 2)}");
    }

    #[test]
    fn inherent_conflict() {
        let mut a = parse_selection_set("{ a(i: 1) _internal_qp_alias_0 }");
        let b = parse_selection_set("{ a(i: 2) }");

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (true, true), false);

        insta::assert_snapshot!(a, @"{a(i: 1) _internal_qp_alias_0 _internal_qp_alias_1: a(i: 2)}");
    }

    #[test]
    fn inherent_conflict_alias() {
        let mut a = parse_selection_set("{ a(i: 1) _internal_qp_alias_0: test }");
        let b = parse_selection_set("{ a(i: 2) }");

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (false, true), false);

        insta::assert_snapshot!(a, @"{a(i: 1) _internal_qp_alias_0: test _internal_qp_alias_1: a(i: 2)}");
    }

    #[test]
    fn multiple_simple_conflicts() {
        let mut a = parse_selection_set("{ a(i: 1) }");
        let b = parse_selection_set("{ a(i: 2) }");
        let c = parse_selection_set("{ a(i: 3) }");

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (false, true), false);
        merger.merge_selection_set(&mut a, &c, (false, true), false);

        insta::assert_snapshot!(a, @"{a(i: 1) _internal_qp_alias_0: a(i: 2) _internal_qp_alias_1: a(i: 3)}");
    }

    #[test]
    fn nested_conflict() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");
        let c = parse_selection_set("{ p { a(i: 3) } }");

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (false, true), false);
        merger.merge_selection_set(&mut a, &c, (false, true), false);

        insta::assert_snapshot!(a, @"{p{a(i: 1) _internal_qp_alias_0: a(i: 2) _internal_qp_alias_1: a(i: 3)}}");
    }

    #[test]
    fn multiple_merge_processes() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");
        let c = parse_selection_set("{ p { a(i: 3) } }");

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (false, true), false);
        let mut merger2 = SafeSelectionSetMerger::default();
        merger2.merge_selection_set(&mut a, &c, (false, true), false);

        insta::assert_snapshot!(a, @"{p{a(i: 1) _internal_qp_alias_0: a(i: 2) _internal_qp_alias_1: a(i: 3)}}");
    }

    #[test]
    fn preferred_side_source() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (false, true), false);

        insta::assert_snapshot!(a, @"{p{a(i: 1) _internal_qp_alias_0: a(i: 2)}}");
    }

    #[test]
    fn preferred_side_target() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");

        let mut merger = SafeSelectionSetMerger::default();
        merger.merge_selection_set(&mut a, &b, (true, false), false);

        insta::assert_snapshot!(a, @"{p{_internal_qp_alias_0: a(i: 1) a(i: 2)}}");
    }

    #[test]
    fn merge_path_nested() {
        let mut a = parse_selection_set("{ p { a(i: 1) } }");
        let b = parse_selection_set("{ p { a(i: 2) } }");

        let mut merger = SafeSelectionSetMerger::default();
        let merge_locations = merger.merge_selection_set(&mut a, &b, (false, true), false);
        assert_eq!(merge_locations.len(), 1);
        insta::assert_snapshot!(merge_locations[0].0, @"p.a");
        insta::assert_snapshot!(merge_locations[0].1, @"_internal_qp_alias_0");
    }
}
