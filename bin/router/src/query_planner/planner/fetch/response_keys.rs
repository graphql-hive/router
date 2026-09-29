//! Response keys for the fields the planner adds on its own: what a `@requires` or a key needs.
//! The client's fields keep their keys.
//!
//! A key belongs to a place in the response, not to a fetch or a fragment. Every fetch writes
//! into the same objects, and fragments on an interface and on its object types meet on one
//! object. So a planner field gets its key once, for the whole operation, from everything the
//! client and other planner fields use at that place. The fetch that writes the field and the
//! fetches that read it all ask here and get the same key.
//!
//! A place is the response keys from the root, without lists, type conditions or conditions.
//! That's coarser than it has to be: it may alias a field that would've been fine, never the
//! other way around.

use std::collections::HashMap;

use crate::query_planner::ast::{
    arguments::ArgumentsMap,
    merge_path::{MergePath, Segment},
    selection_item::SelectionItem,
    selection_set::{FieldSelection, SelectionSet},
};

type Signature = (String, Option<ArgumentsMap>);

#[derive(Debug, Clone, Default)]
pub struct ResponseKeys {
    places: HashMap<Vec<String>, Place>,
}

#[derive(Debug, Clone, Default)]
struct Place {
    /// Who writes each key. More than one when the client uses a key for different fields in
    /// fragments that never meet, or for one field under different conditions.
    occupants: HashMap<String, Vec<Signature>>,
    /// The keys picked for planner fields so far.
    picked: Vec<(Signature, String)>,
}

impl ResponseKeys {
    pub fn from_operation(selection_set: &SelectionSet) -> Self {
        let mut keys = Self::default();
        keys.take_client_keys(&mut Vec::new(), selection_set);
        keys
    }

    fn take_client_keys(&mut self, place: &mut Vec<String>, selection_set: &SelectionSet) {
        for item in &selection_set.items {
            match item {
                SelectionItem::Field(field) => {
                    let key = field.selection_identifier().to_string();
                    self.places
                        .entry(place.clone())
                        .or_default()
                        .occupants
                        .entry(key.clone())
                        .or_default()
                        .push(signature(field));
                    place.push(key);
                    self.take_client_keys(place, &field.selections);
                    place.pop();
                }
                SelectionItem::InlineFragment(fragment) => {
                    self.take_client_keys(place, &fragment.selections)
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    /// The key `field` goes under at `place`. Its own name, unless something else writes that
    /// key there, then the first `_internal_qp_alias_N` nobody uses there.
    pub fn wire_key(&mut self, place: &[String], field: &FieldSelection) -> String {
        if field.name == "__typename" {
            return field.name.clone();
        }

        let signature = signature(field);
        let place = self.places.entry(place.to_vec()).or_default();
        if let Some((_, key)) = place.picked.iter().find(|(picked, _)| *picked == signature) {
            return key.clone();
        }

        let name_is_free = place
            .occupants
            .get(&field.name)
            .is_none_or(|occupants| occupants.iter().all(|occupant| *occupant == signature));
        let key = if name_is_free {
            field.name.clone()
        } else {
            (0..)
                .map(|index| format!("_internal_qp_alias_{index}"))
                .find(|alias| !place.occupants.contains_key(alias))
                .unwrap()
        };

        place
            .occupants
            .entry(key.clone())
            .or_default()
            .push(signature.clone());
        place.picked.push((signature, key.clone()));
        key
    }

    /// `selection_set`, at `place`, the way a fetch writes it: `_internal_qp_alias_0: price`.
    pub fn output(&mut self, place: &[String], selection_set: &SelectionSet) -> SelectionSet {
        self.map(place, selection_set, &|field, key| {
            if key != field.name {
                field.alias = Some(key);
            }
        })
    }

    /// `selection_set`, at `place`, the way a representation reads it:
    /// `price: _internal_qp_alias_0`, the value under the key, put back under the field's name.
    pub fn input(&mut self, place: &[String], selection_set: &SelectionSet) -> SelectionSet {
        self.map(place, selection_set, &|field, key| {
            if key != field.name {
                field.alias = Some(std::mem::replace(&mut field.name, key));
            }
        })
    }

    fn map(
        &mut self,
        place: &[String],
        selection_set: &SelectionSet,
        apply: &dyn Fn(&mut FieldSelection, String),
    ) -> SelectionSet {
        let items = selection_set
            .items
            .iter()
            .map(|item| match item {
                SelectionItem::Field(field) => {
                    let key = self.wire_key(place, field);
                    let mut nested_place = place.to_vec();
                    nested_place.push(key.clone());
                    let mut field = field.with_new_selections(self.map(
                        &nested_place,
                        &field.selections,
                        apply,
                    ));
                    apply(&mut field, key);
                    SelectionItem::Field(field)
                }
                SelectionItem::InlineFragment(fragment) => {
                    let mut fragment = fragment.clone();
                    fragment.selections = self.map(place, &fragment.selections, apply);
                    SelectionItem::InlineFragment(fragment)
                }
                SelectionItem::FragmentSpread(_) => item.clone(),
            })
            .collect();

        SelectionSet { items }
    }
}

/// The place `path` points at: its response keys.
pub fn place_of(path: &MergePath) -> Vec<String> {
    path.inner
        .iter()
        .filter_map(|segment| match segment {
            Segment::Field(field, _, _) => Some(field.response_key().to_string()),
            _ => None,
        })
        .collect()
}

fn signature(field: &FieldSelection) -> Signature {
    (field.name.clone(), field.arguments().cloned())
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::query::{Definition, OperationDefinition};

    use crate::query_planner::{ast::selection_set::SelectionSet, utils::parsing::parse_operation};

    use super::ResponseKeys;

    fn parse(query: &str) -> SelectionSet {
        match parse_operation(query).definitions.first() {
            Some(Definition::Operation(OperationDefinition::SelectionSet(s))) => s.clone().into(),
            _ => panic!("expected a selection set"),
        }
    }

    fn keys(client: &str) -> ResponseKeys {
        ResponseKeys::from_operation(&parse(client))
    }

    #[test]
    fn same_value_keeps_the_name_another_one_gets_an_alias() {
        let mut keys = keys(r#"{ me { price(currency: "GBP") } }"#);
        let me = ["me".to_string()];

        let gbp = keys.output(&me, &parse(r#"{ price(currency: "GBP") }"#));
        let eur = keys.output(&me, &parse(r#"{ price(currency: "EUR") }"#));
        let eur_read = keys.input(&me, &parse(r#"{ ... on Cat { price(currency: "EUR") } }"#));

        assert_eq!(gbp.to_string(), r#"{price(currency: "GBP")}"#);
        assert_eq!(
            eur.to_string(),
            r#"{_internal_qp_alias_0: price(currency: "EUR")}"#
        );
        assert_eq!(
            eur_read.to_string(),
            r#"{...on Cat{price: _internal_qp_alias_0(currency: "EUR")}}"#
        );
    }

    /// The client can use a key twice, for fields in fragments that never meet. Neither of
    /// them can go, and neither can the client's own `_internal_qp_alias_0`.
    #[test]
    fn every_occupant_counts() {
        let mut keys = keys(
            r#"{ things { _internal_qp_alias_0: id ... on A { price(currency: "GBP") } ... on B { price(currency: "USD") } } }"#,
        );
        let things = ["things".to_string()];

        let gbp = keys.output(&things, &parse(r#"{ price(currency: "GBP") }"#));
        assert_eq!(
            gbp.to_string(),
            r#"{_internal_qp_alias_1: price(currency: "GBP")}"#
        );
    }

    /// Below an aliased object, places follow the alias.
    #[test]
    fn nested_places_follow_the_key() {
        let mut keys = keys(r#"{ me { team(role: "user") { name } } }"#);
        let me = ["me".to_string()];

        let admin = keys.output(&me, &parse(r#"{ team(role: "admin") { name } }"#));
        assert_eq!(
            admin.to_string(),
            r#"{_internal_qp_alias_0: team(role: "admin"){name}}"#
        );
        let nested = ["me".to_string(), "_internal_qp_alias_0".to_string()];
        assert_eq!(
            keys.output(&nested, &parse("{ name }")).to_string(),
            "{name}"
        );
    }
}
