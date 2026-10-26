use std::sync::Arc;

use ahash::AHashMap;
use directives::JoinFieldDirective;
use graphql_tools::parser::{
    parse_query,
    query::{
        Definition, OperationDefinition, Selection, SelectionSet,
        SelectionSet as QuerySelectionSet, TypeCondition,
    },
};

use crate::query_planner::{
    ast::{
        normalization::{context::RootTypes, normalize_operation_mut},
        type_aware_selection::TypeAwareSelection,
    },
    state::supergraph_state::{SupergraphDefinition, SupergraphField, SupergraphState},
};

pub(crate) mod definitions;
pub(crate) mod directives;

pub mod authorization;
pub(crate) mod context;
pub mod demand_control;
pub(crate) mod directive_trait;
pub(crate) mod inacessible;
pub(crate) mod join_directive;
pub(crate) mod join_enum_value;
pub(crate) mod join_field;
pub(crate) mod join_graph;
pub(crate) mod join_implements;
pub(crate) mod join_owner;
pub(crate) mod join_type;
pub(crate) mod join_union;

fn normalize_fields_argument_value_mut(
    supergraph: &SupergraphState,
    type_name: &str,
    subgraph_name: &String,
    fields_str: &str,
) -> SelectionSet<'static, String> {
    let selection_set_str = format!("{{{fields_str}}}");
    // TODO: Far from ideal, but we can use the graphql_parser here to get it parsed for us
    let mut parsed_doc = parse_query(&selection_set_str).unwrap().into_static();

    normalize_operation_mut(
        supergraph,
        &mut parsed_doc,
        None,
        Some(RootTypes {
            query: Some(type_name),
            mutation: supergraph.mutation_type.as_deref(),
            subscription: supergraph.subscription_type.as_deref(),
        }),
        Some(subgraph_name),
    )
    .unwrap_or_else(|err| panic!("Normalization error: {err}"));

    match parsed_doc
        .definitions
        .first()
        .expect("failed to parse selection set")
    {
        Definition::Operation(OperationDefinition::SelectionSet(selection_set)) => {
            selection_set.to_owned()
        }
        _ => {
            unreachable!(
                "Internal error: 'fields' string '{{...}}' did not result in a SelectionSet"
            )
        }
    }
}

/// Walks a `sizedFields` selection set top-down, enforcing exactly one field per
/// level and rejecting fragments, appending each field name to `path`.
fn collect_sized_field_path(
    selection_set: &SelectionSet<'_, String>,
    original: &str,
    path: &mut Vec<String>,
) -> Result<(), String> {
    if selection_set.items.is_empty() {
        return Ok(());
    }

    if selection_set.items.len() != 1 {
        return Err(format!(
            "'sizedFields' entry '{original}' must select exactly one field per level, found {}",
            selection_set.items.len()
        ));
    }

    match &selection_set.items[0] {
        Selection::Field(field) => {
            path.push(field.name.clone());
            collect_sized_field_path(&field.selection_set, original, path)
        }
        Selection::FragmentSpread(_) | Selection::InlineFragment(_) => Err(format!(
            "'sizedFields' entry '{original}' must only contain fields, not fragments"
        )),
    }
}

pub struct FederationRules;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct SelectionCacheKey<'a> {
    subgraph_name: &'a str,
    type_name: &'a str,
    selection: &'a str,
}

/// Caches normalized federation field sets while a planner graph is being built.
/// The cache keys borrow the immutable supergraph; cached selections are shared by graph edges.
#[derive(Debug, Default)]
pub(crate) struct CachedFederationRules<'a> {
    selections: AHashMap<SelectionCacheKey<'a>, Arc<QuerySelectionSet<'static, String>>>,
    type_aware_selections: AHashMap<SelectionCacheKey<'a>, Arc<TypeAwareSelection>>,
}

impl<'a> CachedFederationRules<'a> {
    fn parse_selection(
        &mut self,
        supergraph: &'a SupergraphState,
        subgraph_name: &'a String,
        type_name: &'a str,
        selection: &'a str,
    ) -> Arc<QuerySelectionSet<'static, String>> {
        let key = SelectionCacheKey {
            subgraph_name: subgraph_name.as_str(),
            type_name,
            selection,
        };
        if let Some(selection_set) = self.selections.get(&key) {
            return Arc::clone(selection_set);
        }

        let selection_set = Arc::new(normalize_fields_argument_value_mut(
            supergraph,
            type_name,
            subgraph_name,
            selection,
        ));
        self.selections.insert(key, Arc::clone(&selection_set));
        selection_set
    }

    fn parse_type_aware_selection(
        &mut self,
        supergraph: &'a SupergraphState,
        subgraph_name: &'a String,
        type_name: &'a str,
        selection: &'a str,
    ) -> Arc<TypeAwareSelection> {
        let key = SelectionCacheKey {
            subgraph_name: subgraph_name.as_str(),
            type_name,
            selection,
        };
        if let Some(selection) = self.type_aware_selections.get(&key) {
            return Arc::clone(selection);
        }

        let selection_set = self.parse_selection(supergraph, subgraph_name, type_name, selection);
        let selection = Arc::new(TypeAwareSelection {
            type_name: type_name.to_string(),
            selection_set: selection_set.as_ref().clone().into(),
        });
        self.type_aware_selections
            .insert(key, Arc::clone(&selection));
        selection
    }

    pub(crate) fn parse_key(
        &mut self,
        supergraph: &'a SupergraphState,
        subgraph_name: &'a String,
        type_name: &'a str,
        key: &'a str,
    ) -> Arc<TypeAwareSelection> {
        self.parse_type_aware_selection(supergraph, subgraph_name, type_name, key)
    }

    pub(crate) fn parse_requires(
        &mut self,
        supergraph: &'a SupergraphState,
        subgraph_name: &'a String,
        type_name: &'a str,
        requires: &'a str,
    ) -> Arc<TypeAwareSelection> {
        self.parse_type_aware_selection(supergraph, subgraph_name, type_name, requires)
    }

    pub(crate) fn parse_provides(
        &mut self,
        supergraph: &'a SupergraphState,
        join_field: &'a JoinFieldDirective,
        subgraph_name: &'a String,
        type_name: &'a str,
    ) -> Option<Arc<QuerySelectionSet<'static, String>>> {
        let provides = join_field.provides.as_deref()?;
        Some(self.parse_selection(supergraph, subgraph_name, type_name, provides))
    }
}

impl FederationRules {
    pub fn parse_key(
        supergraph: &SupergraphState,
        subgraph_name: &String,
        type_name: &str,
        key: &str,
    ) -> TypeAwareSelection {
        let selection_set =
            normalize_fields_argument_value_mut(supergraph, type_name, subgraph_name, key);
        TypeAwareSelection {
            type_name: type_name.to_string(),
            selection_set: selection_set.into(),
        }
    }

    pub fn parse_context_selection(
        supergraph: &SupergraphState,
        subgraph_name: &String,
        type_name: &str,
        selection: &str,
    ) -> TypeAwareSelection {
        let selection = selection.trim();
        let fields = if let Some(inner) = selection.strip_prefix('{') {
            inner
                .strip_suffix('}')
                .expect("context selection must end with '}'")
        } else {
            selection
        };
        let mut selection_set =
            normalize_fields_argument_value_mut(supergraph, type_name, subgraph_name, fields);
        selection_set.items.retain(|selection| {
            let Selection::InlineFragment(fragment) = selection else {
                return true;
            };
            let Some(TypeCondition::On(condition)) = &fragment.type_condition else {
                return true;
            };
            if condition == type_name {
                return true;
            }
            match &supergraph.definitions[type_name] {
                SupergraphDefinition::Object(_) => false,
                SupergraphDefinition::Interface(_) => supergraph
                    .interface_members(type_name)
                    .is_some_and(|members| members.contains(condition)),
                SupergraphDefinition::Union(union) => union.types.contains(condition),
                _ => false,
            }
        });
        TypeAwareSelection {
            type_name: type_name.to_string(),
            selection_set: selection_set.into(),
        }
    }

    pub fn parse_provides(
        supergraph: &SupergraphState,
        join_field: &JoinFieldDirective,
        subgraph_name: &String,
        type_name: &str,
    ) -> Option<SelectionSet<'static, String>> {
        if let Some(provides) = &join_field.provides {
            return Some(normalize_fields_argument_value_mut(
                supergraph,
                type_name,
                subgraph_name,
                provides,
            ));
        }

        None
    }

    pub fn parse_requires(
        supergraph: &SupergraphState,
        subgraph_name: &String,
        type_name: &str,
        requires: &str,
    ) -> SelectionSet<'static, String> {
        normalize_fields_argument_value_mut(supergraph, type_name, subgraph_name, requires)
    }

    /// Parses a single `@listSize(sizedFields: [...])` entry, written in
    /// selection-set form, into the flat field path it points at:
    ///
    /// - `"field"` → `["field"]`
    /// - `"field { nested }"` → `["field", "nested"]`
    pub fn parse_list_size_sized_fields(
        sized_fields_selection: &str,
    ) -> Result<Vec<String>, String> {
        let selection_set_str = format!("{{{sized_fields_selection}}}");
        let parsed = parse_query::<String>(&selection_set_str).map_err(|err| {
            format!("invalid 'sizedFields' entry '{sized_fields_selection}': {err}")
        })?;

        let Some(Definition::Operation(OperationDefinition::SelectionSet(selection_set))) =
            parsed.definitions.into_iter().next()
        else {
            return Err(format!(
                "'sizedFields' entry '{sized_fields_selection}' is not a valid field selection"
            ));
        };

        let mut path = Vec::new();
        collect_sized_field_path(&selection_set, sized_fields_selection, &mut path)?;
        Ok(path)
    }

    pub fn check_field_subgraph_availability<'a>(
        field: &'a SupergraphField,
        current_subgraph_id: &str,
        parent_definition: &SupergraphDefinition,
    ) -> (bool, Option<&'a JoinFieldDirective>) {
        // A field i available if: it has no @join__field directives at all
        if field.join_field.is_empty() {
            // AND its parent type is available in the subgraph
            return (
                parent_definition
                    .join_types()
                    .iter()
                    .any(|join_type| join_type.graph_id == current_subgraph_id),
                None,
            );
        }

        // Find the relevant join_field and use it to determine availability
        let join_field = field.join_field.iter().find(|join_field| {
            join_field
                .graph_id
                .as_ref()
                .is_some_and(|g| g == current_subgraph_id)
        });

        if let Some(join_field) = join_field {
            return (true, Some(join_field));
        }

        (false, None)
    }
}

#[cfg(test)]
mod tests {
    use crate::query_planner::{
        ast::selection_item::SelectionItem, state::supergraph_state::SupergraphState,
        utils::parsing::parse_schema,
    };

    use super::FederationRules;

    fn context_supergraph() -> SupergraphState {
        SupergraphState::new(&parse_schema(
            r#"
            type Query @join__type(graph: SUBGRAPH) {
                a: A
            }
            interface Node @join__type(graph: SUBGRAPH) @context(name: "subgraph__shared") {
                id: ID!
            }
            type A implements Node
                @join__type(graph: SUBGRAPH)
                @join__implements(graph: SUBGRAPH, interface: "Node")
                @context(name: "subgraph__shared")
                @context(name: "subgraph__shared")
                @context(name: "subgraph__a") {
                id: ID!
                child: Child
            }
            type Child @join__type(graph: SUBGRAPH) {
                code: String
            }
            type B implements Node
                @join__type(graph: SUBGRAPH)
                @join__implements(graph: SUBGRAPH, interface: "Node") {
                id: ID!
                other: String
            }
            type C @join__type(graph: SUBGRAPH) {
                third: String
            }
            union Pair
                @join__type(graph: SUBGRAPH)
                @join__unionMember(graph: SUBGRAPH, member: "A")
                @join__unionMember(graph: SUBGRAPH, member: "B")
                @context(name: "subgraph__shared") = A | B
            "#,
        ))
    }

    #[test]
    fn collects_contexts_on_objects_interfaces_and_unions() {
        let supergraph = context_supergraph();
        assert_eq!(
            supergraph.contexts["subgraph__shared"],
            ["Node", "A", "Pair"]
        );
        assert_eq!(supergraph.contexts["subgraph__a"], ["A"]);
        assert_eq!(supergraph.contexts.len(), 2);
    }

    #[test]
    fn parses_context_selection_with_outer_braces() {
        let supergraph = context_supergraph();
        let subgraph = "SUBGRAPH".to_string();
        let parsed = FederationRules::parse_context_selection(
            &supergraph,
            &subgraph,
            "A",
            " \n { child { code } } \n ",
        );
        assert_eq!(
            parsed,
            FederationRules::parse_key(&supergraph, &subgraph, "A", &"child { code }".to_string())
        );
    }

    #[test]
    fn drops_context_branches_for_other_object_types() {
        let supergraph = context_supergraph();
        let subgraph = "SUBGRAPH".to_string();
        let parsed = FederationRules::parse_context_selection(
            &supergraph,
            &subgraph,
            "A",
            " ... on A { child { code } } ... on B { other }",
        );
        assert_eq!(
            parsed,
            FederationRules::parse_key(&supergraph, &subgraph, "A", &"child { code }".to_string())
        );
        assert!(FederationRules::parse_context_selection(
            &supergraph,
            &subgraph,
            "A",
            " ... on B { other }",
        )
        .selection_set
        .is_empty());
    }

    #[test]
    fn drops_context_branches_outside_interface_and_union_members() {
        let supergraph = context_supergraph();
        let subgraph = "SUBGRAPH".to_string();
        for type_name in ["Node", "Pair"] {
            let parsed = FederationRules::parse_context_selection(
                &supergraph,
                &subgraph,
                type_name,
                " ... on A { child { code } } ... on B { other } ... on C { third }",
            );
            let conditions: Vec<&str> = parsed
                .selection_set
                .items
                .iter()
                .map(|item| match item {
                    SelectionItem::InlineFragment(fragment) => fragment.type_condition.as_str(),
                    _ => panic!("expected a type-conditioned selection"),
                })
                .collect();
            assert_eq!(conditions, ["A", "B"]);
        }
    }

    #[test]
    fn normalizes_abstract_context_type_conditions() {
        let supergraph = context_supergraph();
        let subgraph = "SUBGRAPH".to_string();
        let parsed = FederationRules::parse_context_selection(
            &supergraph,
            &subgraph,
            "A",
            " ... on Node { id }",
        );
        assert_eq!(parsed.type_name, "A");
        assert_eq!(parsed.selection_set.items.len(), 1);
        match &parsed.selection_set.items[0] {
            SelectionItem::InlineFragment(fragment) => {
                assert_eq!(fragment.type_condition, "A");
                assert_eq!(
                    fragment.selections,
                    FederationRules::parse_key(&supergraph, &subgraph, "A", &"id".to_string())
                        .selection_set
                );
            }
            _ => panic!("expected a type-conditioned selection"),
        }
    }
}

#[cfg(test)]
mod cached_rules_tests {
    use super::CachedFederationRules;
    use crate::query_planner::{
        state::supergraph_state::SupergraphState, utils::parsing::parse_schema,
    };
    use std::sync::Arc;

    #[test]
    fn shares_normalized_key_and_requires_selections() {
        let schema = parse_schema(
            r#"
            enum join__Graph {
              A @join__graph(name: "a", url: "")
            }

            type Query @join__type(graph: A) {
              item: Item @join__field(graph: A)
            }

            type Item @join__type(graph: A, key: "id") {
              id: ID @join__field(graph: A)
              name: String @join__field(graph: A, requires: "id")
            }
            "#,
        );
        let supergraph = SupergraphState::new(&schema);
        let (type_name, definition) = supergraph.definitions.get_key_value("Item").unwrap();
        let join_type = definition.join_types().first().unwrap();
        let key = join_type.key.as_ref().unwrap();
        let requires = definition
            .fields()
            .get("name")
            .unwrap()
            .join_field
            .first()
            .unwrap()
            .requires
            .as_ref()
            .unwrap();

        let mut cached_rules = CachedFederationRules::default();
        let key_selection =
            cached_rules.parse_key(&supergraph, &join_type.graph_id, type_name, key);
        let requires_selection =
            cached_rules.parse_requires(&supergraph, &join_type.graph_id, type_name, requires);

        assert!(Arc::ptr_eq(&key_selection, &requires_selection));
    }
}
