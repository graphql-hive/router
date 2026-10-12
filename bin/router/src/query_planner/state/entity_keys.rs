use ahash::{HashMap, HashSet};

use crate::query_planner::{
    ast::{selection_item::SelectionItem, type_aware_selection::TypeAwareSelection},
    federation_spec::FederationRules,
    state::supergraph_state::SupergraphState,
};

/// One subgraph's resolvable `@key`s for one entity type.
#[derive(Debug, Default)]
pub struct TypeKeys {
    /// Each `@key`, parsed, by its `fields`. The planner's entity moves use them.
    pub selections: HashMap<String, TypeAwareSelection>,
    /// The top-level fields of all of them: `id` and `org` for `@key(fields: "id org { id }")`.
    pub fields: HashSet<String>,
}

/// One subgraph's keys, by entity type.
pub type KeysByType = HashMap<String, TypeKeys>;

/// Each subgraph's resolvable `@key`s, by entity type. Every key is parsed once, here.
#[derive(Debug, Default)]
pub struct EntityKeys(HashMap<String, KeysByType>);

impl EntityKeys {
    pub fn new(supergraph: &SupergraphState) -> Self {
        let mut subgraphs: HashMap<String, KeysByType> = HashMap::default();
        for (type_name, definition) in &supergraph.definitions {
            for join_type in definition.join_types() {
                let (true, Some(key)) = (join_type.resolvable, &join_type.key) else {
                    continue;
                };
                let Some(subgraph) = supergraph.known_subgraphs.get(&join_type.graph_id) else {
                    continue;
                };
                let selection =
                    FederationRules::parse_key(supergraph, &join_type.graph_id, type_name, key);

                // A type keyed by `__typename` alone gets no fields.
                let keys = subgraphs
                    .entry(subgraph.clone())
                    .or_default()
                    .entry(type_name.clone())
                    .or_default();

                for item in &selection.selection_set.items {
                    if let SelectionItem::Field(field) = item {
                        if field.name != "__typename" {
                            keys.fields.insert(field.name.clone());
                        }
                    }
                }

                keys.selections.insert(key.clone(), selection);
            }
        }

        Self(subgraphs)
    }

    /// The keys of each entity type in `subgraph`.
    pub fn of_subgraph(&self, subgraph: &str) -> Option<&KeysByType> {
        self.0.get(subgraph)
    }

    /// The parsed `@key(fields: key)` of `type_name` in `subgraph`.
    pub fn selection(
        &self,
        subgraph: &str,
        type_name: &str,
        key: &str,
    ) -> Option<&TypeAwareSelection> {
        self.0.get(subgraph)?.get(type_name)?.selections.get(key)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{EntityKeys, TypeKeys};
    use crate::query_planner::{
        state::supergraph_state::SupergraphState, utils::parsing::parse_schema,
    };

    fn type_keys<'a>(keys: &'a EntityKeys, subgraph: &str, type_name: &str) -> &'a TypeKeys {
        keys.of_subgraph(subgraph)
            .and_then(|types| types.get(type_name))
            .unwrap_or_else(|| panic!("no keys of {type_name} in {subgraph}"))
    }

    fn sorted<'a>(names: impl Iterator<Item = &'a String>) -> Vec<&'a str> {
        let mut names: Vec<&str> = names.map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// `A` has four keys in each subgraph, one of them resolvable. Only that one counts, and
    /// only its top-level fields.
    #[test]
    fn resolvable_keys_of_each_subgraph() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/keys-mashup.supergraph.graphql");
        let sdl = std::fs::read_to_string(path).expect("fixture should be readable");
        let supergraph = SupergraphState::new(&parse_schema(&sdl));
        let keys = &supergraph.entity_keys;

        let in_a = type_keys(keys, "a", "A");
        assert_eq!(sorted(in_a.selections.keys()), ["id"]);
        assert_eq!(sorted(in_a.fields.iter()), ["id"]);

        let in_b = type_keys(keys, "b", "A");
        assert_eq!(
            sorted(in_b.selections.keys()),
            ["id compositeId { two three }"]
        );
        assert_eq!(sorted(in_b.fields.iter()), ["compositeId", "id"]);

        // `B` is an entity in `b` only
        assert!(keys.selection("a", "B", "id").is_none());
        assert_eq!(sorted(type_keys(keys, "b", "B").fields.iter()), ["id"]);
    }

    /// A key of `__typename` alone has no fields: the subgraph finds the entity without any.
    #[test]
    fn typename_key_has_no_fields() {
        let schema = parse_schema(
            r#"
            enum join__Graph {
              CONTENT @join__graph(name: "content", url: "")
              PETSEARCH @join__graph(name: "petsearch", url: "")
            }
            type Query @join__type(graph: CONTENT) {
              favoritesPage: FavoritesPage @join__field(graph: CONTENT)
            }
            type FavoritesPage
              @join__type(graph: CONTENT, key: "__typename")
              @join__type(graph: PETSEARCH, key: "__typename") {
              title: String @join__field(graph: CONTENT)
              petCount: Int @join__field(graph: PETSEARCH)
            }
            "#,
        );
        let supergraph = SupergraphState::new(&schema);
        let keys = &supergraph.entity_keys;

        let in_petsearch = type_keys(keys, "petsearch", "FavoritesPage");
        assert_eq!(sorted(in_petsearch.selections.keys()), ["__typename"]);
        assert!(in_petsearch.fields.is_empty());
        assert!(keys
            .selection("content", "FavoritesPage", "__typename")
            .is_some());
    }
}
