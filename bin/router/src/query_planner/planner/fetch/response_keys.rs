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

use ahash::HashMap;

use crate::query_planner::ast::{
    arguments::ArgumentsMap,
    merge_path::{MergePath, Segment},
    selection_item::SelectionItem,
    selection_set::{FieldSelection, SelectionSet},
};

type Signature = (String, Option<ArgumentsMap>);

/// A place, as its index in `ResponseKeys::places`.
type PlaceId = u32;

const ROOT: PlaceId = 0;

#[derive(Debug, Clone)]
pub struct ResponseKeys {
    /// Every place seen so far, the root first. A place knows its children by key, so finding
    /// one is a lookup per field on the way, with nothing copied.
    places: Vec<Place>,
}

impl Default for ResponseKeys {
    fn default() -> Self {
        Self {
            places: vec![Place::default()],
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Place {
    /// Who writes each key. More than one when the client uses a key for different fields in
    /// fragments that never meet, or for one field under different conditions.
    occupants: HashMap<String, Vec<Signature>>,
    /// The keys picked for planner fields so far.
    picked: Vec<(Signature, String)>,
    /// The places under each key.
    children: HashMap<String, PlaceId>,
}

impl ResponseKeys {
    pub fn from_operation(selection_set: &SelectionSet) -> Self {
        let mut keys = Self::default();
        keys.take_client_keys(ROOT, selection_set);
        keys
    }

    /// The place under `key` at `place`, made on first use.
    fn child(&mut self, place: PlaceId, key: &str) -> PlaceId {
        if let Some(&child) = self.places[place as usize].children.get(key) {
            return child;
        }
        let child = self.places.len() as PlaceId;
        self.places.push(Place::default());
        self.places[place as usize]
            .children
            .insert(key.to_string(), child);
        child
    }

    /// The place `path` points at: its response keys from the root, without lists, type
    /// conditions or conditions.
    fn place_of(&mut self, path: &MergePath) -> PlaceId {
        path.inner
            .iter()
            .fold(ROOT, |place, segment| match segment {
                Segment::Field(field, _, _) => self.child(place, field.response_key()),
                _ => place,
            })
    }

    fn take_client_keys(&mut self, place: PlaceId, selection_set: &SelectionSet) {
        for item in &selection_set.items {
            match item {
                SelectionItem::Field(field) => {
                    let key = field.selection_identifier();
                    let occupants = &mut self.places[place as usize].occupants;
                    match occupants.get_mut(key) {
                        Some(signatures) => signatures.push(signature(field)),
                        None => {
                            occupants.insert(key.to_string(), vec![signature(field)]);
                        }
                    }
                    // a leaf needs no place under it, `child` makes one if it ever does
                    if !field.selections.items.is_empty() {
                        let child = self.child(place, key);
                        self.take_client_keys(child, &field.selections);
                    }
                }
                SelectionItem::InlineFragment(fragment) => {
                    self.take_client_keys(place, &fragment.selections)
                }
                SelectionItem::FragmentSpread(_) => {}
            }
        }
    }

    /// The key `field` goes under at the place `path` points at. Its own name, unless something
    /// else writes that key there, then the first `_internal_qp_alias_N` nobody uses there.
    pub fn wire_key(&mut self, path: &MergePath, field: &FieldSelection) -> String {
        let place = self.place_of(path);
        self.wire_key_at(place, field)
    }

    fn wire_key_at(&mut self, place: PlaceId, field: &FieldSelection) -> String {
        if field.is_typename() {
            return field.name.clone();
        }

        let place = &mut self.places[place as usize];
        // Compared in place, the signature is only built when it gets stored.
        if let Some((_, key)) = place.picked.iter().find(|((name, arguments), _)| {
            *name == field.name && arguments.as_ref() == field.arguments()
        }) {
            return key.clone();
        }

        let signature = signature(field);

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

    /// `selection_set`, at the place `path` points at, the way a fetch writes it:
    /// `_internal_qp_alias_0: price`.
    pub fn output(&mut self, path: &MergePath, selection_set: &SelectionSet) -> SelectionSet {
        let place = self.place_of(path);
        self.map(place, selection_set, &|field, key| {
            if key != field.name {
                field.alias = Some(key);
            }
        })
    }

    /// `selection_set`, at the place `path` points at, the way a representation reads it:
    /// `price: _internal_qp_alias_0`, the value under the key, put back under the field's name.
    pub fn input(&mut self, path: &MergePath, selection_set: &SelectionSet) -> SelectionSet {
        let place = self.place_of(path);
        self.map(place, selection_set, &|field, key| {
            if key != field.name {
                field.alias = Some(std::mem::replace(&mut field.name, key));
            }
        })
    }

    fn map<F: Fn(&mut FieldSelection, String)>(
        &mut self,
        place: PlaceId,
        selection_set: &SelectionSet,
        apply: &F,
    ) -> SelectionSet {
        let items = selection_set
            .items
            .iter()
            .map(|item| match item {
                SelectionItem::Field(field) => {
                    let key = self.wire_key_at(place, field);
                    let selections = if field.selections.items.is_empty() {
                        SelectionSet::default()
                    } else {
                        let child = self.child(place, &key);
                        self.map(child, &field.selections, apply)
                    };
                    let mut field = field.with_new_selections(selections);
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

/// Turns a representation read back into the fetch that writes it: `price: _internal_qp_alias_0`
/// becomes `_internal_qp_alias_0: price`. For when an input moves into an output.
pub fn read_as_written(selection_set: &mut SelectionSet) {
    for item in selection_set.items.iter_mut() {
        match item {
            SelectionItem::Field(field) => {
                if let Some(alias) = field.alias.take() {
                    field.alias = Some(std::mem::replace(&mut field.name, alias));
                }
                read_as_written(&mut field.selections);
            }
            SelectionItem::InlineFragment(fragment) => read_as_written(&mut fragment.selections),
            SelectionItem::FragmentSpread(_) => {}
        }
    }
}

fn signature(field: &FieldSelection) -> Signature {
    (field.name.clone(), field.arguments().cloned())
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::query::{Definition, OperationDefinition};

    use std::rc::Rc;

    use crate::query_planner::{
        ast::{
            merge_path::{FieldPathSegment, MergePath, Segment},
            selection_set::SelectionSet,
        },
        utils::parsing::parse_operation,
    };

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

    /// The path through these response keys.
    fn path(keys: &[&str]) -> MergePath {
        MergePath::new(
            keys.iter()
                .map(|key| {
                    Segment::Field(Rc::new(FieldPathSegment::named(key.to_string())), 0, None)
                })
                .collect(),
        )
    }

    #[test]
    fn same_value_keeps_the_name_another_one_gets_an_alias() {
        let mut keys = keys(r#"{ me { price(currency: "GBP") } }"#);
        let me = path(&["me"]);

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
        let things = path(&["things"]);

        let gbp = keys.output(&things, &parse(r#"{ price(currency: "GBP") }"#));
        assert_eq!(
            gbp.to_string(),
            r#"{_internal_qp_alias_1: price(currency: "GBP")}"#
        );
    }

    #[test]
    fn a_read_turns_back_into_the_write() {
        let mut keys = keys(r#"{ me { price(currency: "GBP") } }"#);
        let me = path(&["me"]);
        let written = keys.output(&me, &parse(r#"{ ... on Cat { price(currency: "EUR") } }"#));
        let mut read = keys.input(&me, &parse(r#"{ ... on Cat { price(currency: "EUR") } }"#));

        super::read_as_written(&mut read);
        assert_eq!(read.to_string(), written.to_string());
    }

    /// Below an aliased object, places follow the alias.
    #[test]
    fn nested_places_follow_the_key() {
        let mut keys = keys(r#"{ me { team(role: "user") { name } } }"#);
        let me = path(&["me"]);

        let admin = keys.output(&me, &parse(r#"{ team(role: "admin") { name } }"#));
        assert_eq!(
            admin.to_string(),
            r#"{_internal_qp_alias_0: team(role: "admin"){name}}"#
        );
        let nested = path(&["me", "_internal_qp_alias_0"]);
        assert_eq!(
            keys.output(&nested, &parse("{ name }")).to_string(),
            "{name}"
        );
    }
}
