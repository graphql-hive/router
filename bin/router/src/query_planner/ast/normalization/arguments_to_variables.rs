use crate::query_planner::ast::operation::{OperationDefinition, VariableDefinition};
use crate::query_planner::ast::selection_item::SelectionItem;
use crate::query_planner::ast::selection_set::SelectionSet;
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
    operation: &mut OperationDefinition,
    supergraph: &SupergraphState,
) -> Vec<ExtractedVariable> {
    let root_type = match operation.operation_kind {
        Some(OperationKind::Mutation) => supergraph.mutation_type.as_deref(),
        Some(OperationKind::Subscription) => supergraph.subscription_type.as_deref(),
        _ => Some(supergraph.query_type.as_str()),
    };
    let Some(root_type) = root_type else {
        return Vec::new();
    };

    let mut extractor = Extractor {
        supergraph,
        client_variables: operation
            .variable_definitions
            .as_deref()
            .unwrap_or_default(),
        next_index: 0,
        extracted: Vec::new(),
        seen: Default::default(),
    };
    extractor.selection_set(&mut operation.selection_set, root_type);
    let extracted = extractor.extracted;

    if extracted.is_empty() {
        return Vec::new();
    }

    let definitions = operation.variable_definitions.get_or_insert_with(Vec::new);
    extracted
        .into_iter()
        .map(|(variable_type, extracted)| {
            definitions.push(VariableDefinition {
                name: extracted.name.clone(),
                variable_type,
                default_value: None,
            });
            extracted
        })
        .collect()
}

struct Extractor<'a> {
    supergraph: &'a SupergraphState,
    client_variables: &'a [VariableDefinition],
    next_index: usize,
    extracted: Vec<(TypeNode, ExtractedVariable)>,
    seen: std::collections::HashMap<(TypeNode, Value), String>,
}

impl Extractor<'_> {
    fn selection_set(&mut self, selection_set: &mut SelectionSet, parent_type: &str) {
        for item in &mut selection_set.items {
            match item {
                SelectionItem::Field(field) => {
                    let Some(field_def) = self
                        .supergraph
                        .definitions
                        .get(parent_type)
                        .and_then(|def| def.fields().get(&field.name))
                    else {
                        continue;
                    };
                    for (arg_name, value) in field.arguments.iter_mut().flat_map(|a| a.iter_mut()) {
                        let Some(arg_type) = field_def.argument_types.get(arg_name) else {
                            continue;
                        };
                        // `{ id: $id, name: "a" }` stays as it is. Pulling out the literal
                        // parts one by one isn't worth it.
                        if value.contains_variable() {
                            continue;
                        }
                        let literal = std::mem::replace(value, Value::Null);
                        let argument = || format!("{parent_type}.{}({arg_name}:)", field.name);
                        *value = Value::Variable(self.variable_for(arg_type, literal, argument));
                    }
                    self.selection_set(&mut field.selections, field_def.field_type.inner_type());
                }
                SelectionItem::InlineFragment(fragment) => {
                    self.selection_set(&mut fragment.selections, &fragment.type_condition);
                }
                SelectionItem::FragmentSpread(_) => {
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
        let key = (variable_type.clone(), value);
        if let Some(name) = self.seen.get(&key) {
            return name.clone();
        }
        let name = loop {
            let name = format!("_v{}", self.next_index);
            self.next_index += 1;
            if !self.client_variables.iter().any(|def| def.name == name) {
                break name;
            }
        };
        self.extracted.push((
            key.0.clone(),
            ExtractedVariable {
                name: name.clone(),
                value: key.1.clone(),
                argument: argument(),
            },
        ));
        self.seen.insert(key, name.clone());
        name
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
        let mut operation =
            normalize_operation(&supergraph, &parse_query(query).expect("to parse"), None)
                .unwrap()
                .operation;
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
            let mut operation =
                normalize_operation(&supergraph, &parse_query(query).expect("to parse"), None)
                    .unwrap()
                    .operation;
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
