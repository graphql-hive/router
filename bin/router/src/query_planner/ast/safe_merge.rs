use std::collections::HashMap;

use tracing::trace;

use crate::query_planner::{
    ast::{
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

const ALIAS_PREFIX: &str = "_internal_qp_alias_";

/// Two fields with the same response key that aren't the same field. The plan's own fields
/// got keys of their own before anything was merged (see `response_keys`), so these are both
/// the client's, and the merge can't happen.
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

    /// Adds `source` to `target`. Fails when a field of one has the response key of another
    /// field in the other, anywhere they end up on the same objects.
    pub fn merge_selection_set(
        &self,
        target: &mut SelectionSet,
        source: &SelectionSet,
        as_first: bool,
    ) -> Result<(), UnresolvableConflict> {
        merge_items(target, source, as_first)?;

        // The merge above only compares items at the same level, with the same conditions.
        // Fields that land in different fragments, or have different `@include`/`@skip`,
        // are copied next to each other, so check the whole result.
        self.check_conflicts(target)
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
}

fn merge_items(
    target: &mut SelectionSet,
    source: &SelectionSet,
    as_first: bool,
) -> Result<(), UnresolvableConflict> {
    let mut copies = Vec::new();

    for source_item in source.items.iter() {
        let mut merged = false;

        for target_item in target.items.iter_mut() {
            match (source_item, target_item) {
                (SelectionItem::Field(source_field), SelectionItem::Field(target_field))
                    if source_field.selection_identifier()
                        == target_field.selection_identifier()
                        && source_field.include_if == target_field.include_if
                        && source_field.skip_if == target_field.skip_if =>
                {
                    if source_field.arguments_hash() != target_field.arguments_hash()
                        || source_field.alias != target_field.alias
                    {
                        trace!(
                            "found a conflicting field '{}' ({} != {})",
                            source_field.name,
                            source_field.arguments_hash(),
                            target_field.arguments_hash(),
                        );
                        return Err(UnresolvableConflict(source_field.name.clone()));
                    }
                    merge_items(
                        &mut target_field.selections,
                        &source_field.selections,
                        as_first,
                    )?;
                    merged = true;
                    break;
                }
                (
                    SelectionItem::InlineFragment(source_fragment),
                    SelectionItem::InlineFragment(target_fragment),
                ) if source_fragment.type_condition == target_fragment.type_condition
                    && source_fragment.include_if == target_fragment.include_if
                    && source_fragment.skip_if == target_fragment.skip_if =>
                {
                    merge_items(
                        &mut target_fragment.selections,
                        &source_fragment.selections,
                        as_first,
                    )?;
                    merged = true;
                    break;
                }
                _ => {}
            }
        }

        if !merged {
            copies.push(source_item.clone());
        }
    }

    for item in copies {
        if as_first {
            target.items.insert(0, item);
        } else {
            target.items.push(item);
        }
    }

    Ok(())
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

    fn merge(a: &str, b: &str) -> Result<String, String> {
        let mut a = parse_selection_set(a);
        new_merger()
            .merge_selection_set(&mut a, &parse_selection_set(b), false)
            .map(|_| a.to_string())
            .map_err(|conflict| conflict.0)
    }

    #[test]
    fn merges_fields() {
        insta::assert_snapshot!(merge("{ a }", "{ b }").unwrap(), @"{a b}");
        insta::assert_snapshot!(merge("{ a }", "{ a }").unwrap(), @"{a}");
        insta::assert_snapshot!(merge("{ p { a } }", "{ p { b } }").unwrap(), @"{p{a b}}");
    }

    #[test]
    fn conflict_is_an_error() {
        assert!(merge("{ a { b(x: 1) } }", "{ a { b(x: 2) } }").is_err());
        assert!(merge("{ a(i: 1) }", "{ a(i: 2) }").is_err());
        assert!(merge("{ a }", "{ a: b }").is_err());
    }

    #[test]
    fn conflict_under_different_conditions_is_an_error() {
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
}
