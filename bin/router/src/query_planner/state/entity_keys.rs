use ahash::HashMap;

use crate::query_planner::{
    ast::selection_item::SelectionItem, federation_spec::FederationRules,
    state::supergraph_state::SupergraphState,
};

/// The top-level fields of one resolvable `@key`, without `__typename`: `id` and `org` for
/// `@key(fields: "id org { id }")`. The subgraph can't find an entity by this key without all of
/// them. Empty for a key of `__typename` alone.
pub type KeyFields = Vec<String>;

/// One type's keys in one subgraph.
pub type TypeKeys = Vec<KeyFields>;

/// One subgraph's keys, by entity type.
pub type KeysByType = HashMap<String, TypeKeys>;

/// Each subgraph's resolvable `@key`s, by entity type.
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
                let fields = selection
                    .selection_set
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        SelectionItem::Field(field) if field.name != "__typename" => {
                            Some(field.name.clone())
                        }
                        _ => None,
                    })
                    .collect();

                subgraphs
                    .entry(subgraph.clone())
                    .or_default()
                    .entry(type_name.clone())
                    .or_default()
                    .push(fields);
            }
        }

        Self(subgraphs)
    }

    /// The keys of each entity type in `subgraph`.
    pub fn of_subgraph(&self, subgraph: &str) -> Option<&KeysByType> {
        self.0.get(subgraph)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{EntityKeys, TypeKeys};
    use crate::query_planner::{
        state::supergraph_state::SupergraphState, utils::parsing::parse_schema,
    };

    /// The keys of `type_name` in `subgraph`, each with its fields sorted.
    fn keys_of<'a>(keys: &'a EntityKeys, subgraph: &str, type_name: &str) -> Vec<Vec<&'a str>> {
        let type_keys: &TypeKeys = keys
            .of_subgraph(subgraph)
            .and_then(|types| types.get(type_name))
            .unwrap_or_else(|| panic!("no keys of {type_name} in {subgraph}"));
        let mut keys: Vec<Vec<&str>> = type_keys
            .iter()
            .map(|fields| {
                let mut fields: Vec<&str> = fields.iter().map(String::as_str).collect();
                fields.sort_unstable();
                fields
            })
            .collect();
        keys.sort();
        keys
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

        assert_eq!(keys_of(keys, "a", "A"), [["id"]]);
        // `id compositeId { two three }`
        assert_eq!(keys_of(keys, "b", "A"), [["compositeId", "id"]]);

        // `B` is an entity in `b` only
        assert!(keys
            .of_subgraph("a")
            .is_some_and(|types| !types.contains_key("B")));
        assert_eq!(keys_of(keys, "b", "B"), [["id"]]);
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

        let no_fields: [[&str; 0]; 1] = [[]];
        assert_eq!(keys_of(keys, "petsearch", "FavoritesPage"), no_fields);
        assert_eq!(keys_of(keys, "content", "FavoritesPage"), no_fields);
    }
}
