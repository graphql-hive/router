use indexmap::map::Entry;
use indexmap::IndexMap;

use crate::query_planner::ast::flat::arena::{Arena, ListId};
use crate::query_planner::ast::flat::plan_document::PlanDocument;
use crate::query_planner::ast::flat::Kind;
use crate::query_planner::ast::operation::VariableDefinition;
use crate::query_planner::ast::value::Value;
use crate::query_planner::state::supergraph_state::{OperationKind, SupergraphState, TypeNode};

/// A hard-coded argument value that was turned into a variable.
#[derive(Debug, Hash)]
pub struct ExtractedVariable {
    pub name: String,
    pub value: Value,
    /// Where the value was written, like `Query.user(id:)`. The client never wrote `$_v0`,
    /// so errors about its value point here instead.
    pub argument: String,
}

/// Turns hard-coded field argument values into variables, so `{ user(id: "1") }` and
/// `{ user(id: "2") }` both become `query($_v0: ID!) { user(id: $_v0) }` and share one plan.
/// Gives back the values the new variables should hold.
///
/// Run it on a fully normalized operation. Earlier passes merge fields by comparing their
/// arguments, and fresh variables would make equal arguments look different.
///
/// Only field arguments are touched. Directive arguments like `@include(if: false)` are
/// resolved by normalization already, and variables in default values aren't allowed.
pub fn arguments_to_variables(
    document: &mut PlanDocument,
    supergraph: &SupergraphState,
) -> Vec<ExtractedVariable> {
    let root_type = match document.kind {
        Some(OperationKind::Mutation) => supergraph.mutation_type.as_deref(),
        Some(OperationKind::Subscription) => supergraph.subscription_type.as_deref(),
        _ => Some(supergraph.query_type.as_str()),
    };
    let Some(root_type) = root_type else {
        return Vec::new();
    };

    let mut extractor = Extractor {
        supergraph,
        client_variables: document.variables.as_deref().unwrap_or_default(),
        next_index: 0,
        extracted: IndexMap::new(),
    };
    extractor.selection_set(&mut document.arena, document.selections, root_type);
    let extracted = extractor.extracted;

    if extracted.is_empty() {
        return Vec::new();
    }

    let definitions = document.variables.get_or_insert_with(Vec::new);
    extracted
        .into_iter()
        .map(|((variable_type, value), (name, argument))| {
            definitions.push(VariableDefinition {
                name: name.clone(),
                variable_type,
                default_value: None,
            });
            ExtractedVariable {
                name,
                value,
                argument,
            }
        })
        .collect()
}

struct Extractor<'a> {
    supergraph: &'a SupergraphState,
    client_variables: &'a [VariableDefinition],
    next_index: usize,
    /// The name and argument of every extracted value, in the order they were found
    extracted: IndexMap<(TypeNode, Value), (String, String)>,
}

impl Extractor<'_> {
    /// Every node of a normalized document is its own, no two lists share one, so a field's
    /// arguments are replaced in place.
    fn selection_set(&mut self, arena: &mut Arena, list: ListId, parent_type: &str) {
        for index in 0..arena.items(list).len() {
            let id = arena.items(list)[index];
            let node = *arena.node(id);
            match node.kind {
                Kind::Field => {
                    let name = arena.str(node.name);
                    let Some(field_def) = self
                        .supergraph
                        .definitions
                        .get(parent_type)
                        .and_then(|def| def.fields().get(name))
                    else {
                        continue;
                    };
                    if let Some(arguments) = arena.args(node.args) {
                        let mut arguments = arguments.clone();
                        let mut changed = false;
                        for (arg_name, value) in arguments.iter_mut() {
                            let Some(arg_type) = field_def.argument_types.get(arg_name) else {
                                continue;
                            };
                            // `{ id: $id, name: "a" }` stays as it is. Pulling out the literal
                            // parts one by one isn't worth it.
                            if value.contains_variable() {
                                continue;
                            }
                            let literal = std::mem::replace(value, Value::Null);
                            let argument = || format!("{parent_type}.{name}({arg_name}:)");
                            *value =
                                Value::Variable(self.variable_for(arg_type, literal, argument));
                            changed = true;
                        }
                        if changed {
                            arena.node_mut(id).args = arena.intern_args(&arguments);
                        }
                    }
                    self.selection_set(arena, node.children, field_def.field_type.inner_type());
                }
                Kind::InlineFragment => {
                    let type_condition = arena.str(node.name).to_string();
                    self.selection_set(arena, node.children, &type_condition);
                }
                Kind::FragmentSpread => {
                    // Fragment spreads are removed earlier in normalization.
                }
            }
        }
    }

    /// Equal values of the same type share a variable. Normalization copies fields into every
    /// possible type of an interface, and the planner can only fold the copies back together
    /// if their arguments still match.
    fn variable_for(
        &mut self,
        variable_type: &TypeNode,
        value: Value,
        argument: impl FnOnce() -> String,
    ) -> String {
        match self.extracted.entry((variable_type.clone(), value)) {
            Entry::Occupied(entry) => entry.get().0.clone(),
            Entry::Vacant(entry) => {
                let name = loop {
                    let name = format!("_v{}", self.next_index);
                    self.next_index += 1;
                    if !self.client_variables.iter().any(|def| def.name == name) {
                        break name;
                    }
                };
                entry.insert((name.clone(), argument()));
                name
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use graphql_tools::parser::parse_query;

    use super::arguments_to_variables;
    use crate::query_planner::ast::normalization::normalize_operation;
    use crate::query_planner::state::supergraph_state::SupergraphState;
    use crate::query_planner::utils::parsing::parse_schema;

    const SCHEMA: &str = r#"
      enum Status { ACTIVE INACTIVE }

      input Filter { status: Status, tags: [String!] }

      interface Node { id: ID! }

      type User implements Node {
        id: ID!
        name(upper: Boolean): String
        friends(first: Int!, filter: Filter): [User!]!
      }

      type Post implements Node {
        id: ID!
        title(upper: Boolean): String
      }

      type Query {
        user(id: ID!): User
        node(id: ID!): Node
        users(ids: [ID!]!, status: Status, filter: Filter): [User!]!
      }
    "#;

    fn extract(query: &'static str) -> String {
        let supergraph = SupergraphState::new(&parse_schema(SCHEMA));
        let mut operation = std::sync::Arc::unwrap_or_clone(
            normalize_operation(&supergraph, &parse_query(query).expect("to parse"), None)
                .unwrap()
                .document,
        );
        let values = arguments_to_variables(&mut operation, &supergraph);
        let values: Vec<String> = values
            .iter()
            .map(|v| format!("{} = {} at {}", v.name, v.value, v.argument))
            .collect();
        let operation = operation.to_string();
        let pretty = parse_query::<&str>(&operation).expect("to parse");
        format!("{pretty}{}", values.join("\n"))
    }

    #[test]
    fn different_literals_end_up_as_the_same_operation() {
        let supergraph = SupergraphState::new(&parse_schema(SCHEMA));
        let hash = |query: &'static str| {
            let mut operation = std::sync::Arc::unwrap_or_clone(
                normalize_operation(&supergraph, &parse_query(query).expect("to parse"), None)
                    .unwrap()
                    .document,
            );
            arguments_to_variables(&mut operation, &supergraph);
            operation.hash()
        };
        assert_eq!(
            hash(r#"{ user(id: "1") { name } }"#),
            hash(r#"{ user(id: "2") { name } }"#)
        );
        assert_eq!(
            hash(r#"{ user(id: "1") { name } }"#),
            hash(r#"{ user(id: 3) { name } }"#)
        );
    }

    #[test]
    fn scalars_enums_lists_and_objects() {
        insta::assert_snapshot!(
            extract(
                r#"{
                  user(id: 1) {
                    name(upper: true)
                    friends(first: 10, filter: { status: ACTIVE, tags: ["a"] }) { id }
                  }
                  users(ids: "2", status: INACTIVE) { id }
                }"#,
            ),
            @r#"
        query($_v0: ID!, $_v1: Boolean, $_v2: Filter, $_v3: Int!, $_v4: [ID!]!, $_v5: Status) {
          user(id: $_v0) {
            name(upper: $_v1)
            friends(filter: $_v2, first: $_v3) {
              id
            }
          }
          users(ids: $_v4, status: $_v5) {
            id
          }
        }
        _v0 = 1 at Query.user(id:)
        _v1 = true at User.name(upper:)
        _v2 = {status: ACTIVE, tags: ["a"]} at User.friends(filter:)
        _v3 = 10 at User.friends(first:)
        _v4 = "2" at Query.users(ids:)
        _v5 = INACTIVE at Query.users(status:)
        "#
        );
    }

    #[test]
    fn leaves_client_variables_alone_and_avoids_their_names() {
        insta::assert_snapshot!(
            extract(
                r#"query ($_v0: ID!, $first: Int!) {
                  user(id: $_v0) {
                    friends(first: $first, filter: { status: $_v0 }) { id }
                    name(upper: false)
                  }
                }"#,
            ),
            @r"
        query($_v0: ID!, $first: Int!, $_v1: Boolean) {
          user(id: $_v0) {
            friends(filter: {status: $_v0}, first: $first) {
              id
            }
            name(upper: $_v1)
          }
        }
        _v1 = false at User.name(upper:)
        "
        );
    }

    #[test]
    fn copies_of_a_field_share_a_variable() {
        insta::assert_snapshot!(
            extract(
                r#"{
                  node(id: "1") { ... on User { name(upper: true) } ... on Post { title(upper: true) } }
                  a: user(id: "1") { id }
                }"#,
            ),
            @r#"
        query($_v0: ID!, $_v1: Boolean) {
          node(id: $_v0) {
            ... on User {
              name(upper: $_v1)
            }
            ... on Post {
              title(upper: $_v1)
            }
          }
          a: user(id: $_v0) {
            id
          }
        }
        _v0 = "1" at Query.node(id:)
        _v1 = true at User.name(upper:)
        "#
        );
    }
}
