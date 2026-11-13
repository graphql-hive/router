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

use crate::query_planner::ast::flat::{
    arena::{Arena, ListId},
    selection_tree::{Sel, SelectionTree},
    Kind,
};

use crate::query_planner::ast::{
    arguments::ArgumentsMap,
    merge_path::{MergePath, Segment},
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
    /// The client's keys, from the flat operation the walker reads.
    pub fn from_selection_tree(tree: &SelectionTree) -> Self {
        fn take<'a>(
            keys: &mut ResponseKeys,
            place: &mut Vec<String>,
            items: impl Iterator<Item = Sel<'a>>,
        ) {
            for item in items {
                match item.kind() {
                    Kind::Field => {
                        let attributes = item.attributes();
                        let key = attributes
                            .and_then(|a| a.alias.clone())
                            .unwrap_or_else(|| item.name().to_string());
                        let arguments = attributes
                            .and_then(|a| a.arguments.as_ref())
                            .filter(|a| !a.is_empty())
                            .cloned();
                        let signature = (item.name().to_string(), arguments);
                        keys.place_mut(place)
                            .occupants
                            .entry(key.clone())
                            .or_default()
                            .push(signature);
                        place.push(key);
                        take(keys, place, item.children());
                        place.pop();
                    }
                    Kind::InlineFragment => take(keys, place, item.children()),
                    Kind::FragmentSpread => {}
                }
            }
        }
        let mut keys = Self::default();
        take(&mut keys, &mut Vec::new(), tree.roots());
        keys
    }

    /// The place, made on first use. Looked up by slice, so the key is copied only once.
    fn place_mut(&mut self, place: &[String]) -> &mut Place {
        if !self.places.contains_key(place) {
            self.places.insert(place.to_vec(), Place::default());
        }
        self.places.get_mut(place).expect("inserted above")
    }

    /// The key the field `name(arguments)` goes under at `place`. Its own name, unless something
    /// else writes that key there, then the first `_internal_qp_alias_N` nobody uses there.
    pub fn wire_key(
        &mut self,
        place: &[String],
        name: &str,
        arguments: Option<&ArgumentsMap>,
    ) -> String {
        if name == "__typename" {
            return name.to_string();
        }
        // like `FieldSelection::arguments()`, empty arguments are no arguments
        let arguments = arguments.filter(|a| !a.is_empty());

        let place = self.place_mut(place);
        // Compared in place, the signature is only built when it gets stored.
        if let Some((_, key)) = place
            .picked
            .iter()
            .find(|((n, a), _)| n == name && a.as_ref() == arguments)
        {
            return key.clone();
        }

        let signature: Signature = (name.to_string(), arguments.cloned());

        let name_is_free = place
            .occupants
            .get(name)
            .is_none_or(|occupants| occupants.iter().all(|occupant| *occupant == signature));
        let key = if name_is_free {
            name.to_string()
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

    /// `list`, at `place`, changed in place to the way a fetch writes it:
    /// `_internal_qp_alias_0: price`.
    pub fn output(&mut self, arena: &mut Arena, place: &[String], list: ListId) {
        self.map(arena, &mut place.to_vec(), list, false)
    }

    /// `list`, at `place`, changed in place to the way a representation reads it:
    /// `price: _internal_qp_alias_0`, the value under the key, put back under the field's name.
    pub fn input(&mut self, arena: &mut Arena, place: &[String], list: ListId) {
        self.map(arena, &mut place.to_vec(), list, true)
    }

    /// `place` is a stack: a field's key goes on it while its selections are mapped.
    fn map(&mut self, arena: &mut Arena, place: &mut Vec<String>, list: ListId, input: bool) {
        for index in 0..arena.items(list).len() {
            let id = arena.items(list)[index];
            let node = *arena.node(id);
            match node.kind {
                Kind::Field => {
                    let key = self.wire_key(place, arena.str(node.name), arena.args(node.args));
                    place.push(key);
                    self.map(arena, place, node.children, input);
                    let key = place.pop().expect("pushed above");
                    if key != arena.str(node.name) {
                        let key = arena.intern(&key);
                        let node = arena.node_mut(id);
                        if input {
                            node.alias = std::mem::replace(&mut node.name, key);
                        } else {
                            node.alias = key;
                        }
                    }
                }
                Kind::InlineFragment => self.map(arena, place, node.children, input),
                Kind::FragmentSpread => {}
            }
        }
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

#[cfg(test)]
mod tests {
    use graphql_tools::parser::query::{Definition, OperationDefinition};

    use crate::query_planner::{
        ast::{
            flat::{arena::Arena, plan_document::DisplayList, selection_tree::SelectionTree},
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
        ResponseKeys::from_selection_tree(&SelectionTree::from_tree(&parse(client)))
    }

    fn output(keys: &mut ResponseKeys, place: &[String], query: &str) -> String {
        let mut arena = Arena::new();
        let list = arena.import(&parse(query));
        keys.output(&mut arena, place, list);
        DisplayList(&arena, list).to_string()
    }

    fn input(keys: &mut ResponseKeys, place: &[String], query: &str) -> String {
        let mut arena = Arena::new();
        let list = arena.import(&parse(query));
        keys.input(&mut arena, place, list);
        DisplayList(&arena, list).to_string()
    }

    #[test]
    fn same_value_keeps_the_name_another_one_gets_an_alias() {
        let mut keys = keys(r#"{ me { price(currency: "GBP") } }"#);
        let me = ["me".to_string()];

        let gbp = output(&mut keys, &me, r#"{ price(currency: "GBP") }"#);
        let eur = output(&mut keys, &me, r#"{ price(currency: "EUR") }"#);
        let eur_read = input(
            &mut keys,
            &me,
            r#"{ ... on Cat { price(currency: "EUR") } }"#,
        );

        assert_eq!(gbp, r#"{price(currency: "GBP")}"#);
        assert_eq!(eur, r#"{_internal_qp_alias_0: price(currency: "EUR")}"#);
        assert_eq!(
            eur_read,
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

        let gbp = output(&mut keys, &things, r#"{ price(currency: "GBP") }"#);
        assert_eq!(gbp, r#"{_internal_qp_alias_1: price(currency: "GBP")}"#);
    }

    #[test]
    fn a_read_turns_back_into_the_write() {
        let mut keys = keys(r#"{ me { price(currency: "GBP") } }"#);
        let me = ["me".to_string()];
        let written = output(
            &mut keys,
            &me,
            r#"{ ... on Cat { price(currency: "EUR") } }"#,
        );

        let mut arena = Arena::new();
        let list = arena.import(&parse(r#"{ ... on Cat { price(currency: "EUR") } }"#));
        keys.input(&mut arena, &me, list);
        arena.read_as_written(list);
        assert_eq!(DisplayList(&arena, list).to_string(), written);
    }

    /// Below an aliased object, places follow the alias.
    #[test]
    fn nested_places_follow_the_key() {
        let mut keys = keys(r#"{ me { team(role: "user") { name } } }"#);
        let me = ["me".to_string()];

        let admin = output(&mut keys, &me, r#"{ team(role: "admin") { name } }"#);
        assert_eq!(
            admin,
            r#"{_internal_qp_alias_0: team(role: "admin"){name}}"#
        );
        let nested = ["me".to_string(), "_internal_qp_alias_0".to_string()];
        assert_eq!(output(&mut keys, &nested, "{ name }"), "{name}");
    }
}
