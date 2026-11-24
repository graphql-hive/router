//! How `@defer` comes out of normalization. The marker only shows in debug output,
//! `Display` (used for subgraph queries) never prints it.

use crate::query_planner::ast::normalization::normalize_operation;
use crate::query_planner::state::supergraph_state::SupergraphState;
use crate::query_planner::utils::parsing::{parse_operation, parse_schema};
use crate::query_planner::utils::pretty_display::PrettyDisplay;

const SCHEMA: &str = r#"
type Query @join__type(graph: A) {
  me: User
  node: Node
  users: [User]
}

interface Node @join__type(graph: A) {
  id: ID!
}

type User implements Node
  @join__type(graph: A, key: "id")
  @join__implements(graph: A, interface: "Node") {
  id: ID!
  name: String
  email: String
  friends: [User]
}

type Post implements Node
  @join__type(graph: A, key: "id")
  @join__implements(graph: A, interface: "Node") {
  id: ID!
  title: String
}
"#;

struct Pretty<'a, T: PrettyDisplay>(&'a T);

impl<T: PrettyDisplay> std::fmt::Display for Pretty<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.pretty_fmt(f, 0)
    }
}

fn normalize(query: &str) -> (String, String) {
    let schema = parse_schema(SCHEMA);
    let supergraph = SupergraphState::new(&schema);
    let doc = normalize_operation(&supergraph, &parse_operation(query), None).unwrap();
    let op = doc.executable_operation();
    (
        format!("{}", Pretty(&op.selection_set)),
        op.selection_set.to_string(),
    )
}

#[test]
fn marker_on_inline_fragment_without_type_condition() {
    let (pretty, display) = normalize(r#"{ me { id ... @defer(label: "a") { name } } }"#);
    insta::assert_snapshot!(pretty, @r#"
    me {
      id
      ... on User @defer(label: "a") {
        name
      }
    }
    "#);
    assert!(!display.contains("defer"), "{display}");
}

#[test]
fn marker_on_fragment_spread() {
    let (pretty, _) = normalize(
        r#"query ($v: Boolean!) { me { id ...F @defer(if: $v) } } fragment F on User { name }"#,
    );
    insta::assert_snapshot!(pretty, @r"
    me {
      id
      ... on User @defer(if: $v) {
        name
      }
    }
    ");
}

#[test]
fn literal_false_drops_the_marker() {
    let (pretty, _) = normalize(r#"{ me { id ... @defer(if: false) { name } } }"#);
    insta::assert_snapshot!(pretty, @r"
    me {
      id
      ... on User {
        name
      }
    }
    ");
}

#[test]
fn different_labels_stay_apart() {
    let (pretty, _) = normalize(
        r#"{ me { id ... @defer(label: "a") { name } ... @defer(label: "b") { email } } }"#,
    );
    insta::assert_snapshot!(pretty, @r#"
    me {
      id
      ... on User @defer(label: "a") {
        name
      }
      ... on User @defer(label: "b") {
        email
      }
    }
    "#);
}

#[test]
fn parent_defer_is_copied_into_conditional_children() {
    let (pretty, _) = normalize(
        r#"query ($v: Boolean!) { node { ... @defer(label: "a") { id ... on User @include(if: $v) { name } } } }"#,
    );
    insta::assert_snapshot!(pretty, @r#"
    node {
      ... on Post @defer(label: "a") {
        id
      }
      ... on User @defer(label: "a") {
        id
      }
      ... on User @include(if: $v) @defer(label: "a") {
        name
      }
    }
    "#);
}

#[test]
fn defer_on_abstract_type_is_expanded() {
    let (pretty, _) = normalize(r#"{ node { id ... on Node @defer { id } } }"#);
    insta::assert_snapshot!(pretty, @r"
    node {
      id
      ... on Node @defer {
        id
      }
    }
    ");
}

#[test]
fn nested_defers() {
    let (pretty, _) = normalize(
        r#"{ me { id ... @defer(label: "outer") { name friends { id ... @defer(label: "inner") { email } } } } }"#,
    );
    insta::assert_snapshot!(pretty, @r#"
    me {
      id
      ... on User @defer(label: "outer") {
        name
        friends {
          id
          ... on User @defer(label: "inner") {
            email
          }
        }
      }
    }
    "#);
}

#[test]
fn defer_without_type_condition_on_interface() {
    let (pretty, _) = normalize(r#"{ node { id ... @defer(label: "a") { id } } }"#);
    insta::assert_snapshot!(pretty, @r#"
    node {
      id
      ... on Post @defer(label: "a") {
        id
      }
      ... on User @defer(label: "a") {
        id
      }
    }
    "#);
}

#[test]
fn unlabeled_defers_stay_apart() {
    let (pretty, _) = normalize(
        r#"{ me { id ... @defer { name } ... @defer { email ... @defer { friends { id } } } } }"#,
    );
    insta::assert_snapshot!(pretty, @r"
    me {
      id
      ... on User @defer {
        name
      }
      ... on User @defer {
        email
        ... on User @defer {
          friends {
            id
          }
        }
      }
    }
    ");
}

#[test]
fn formatting_does_not_change_the_hash() {
    let schema = parse_schema(SCHEMA);
    let supergraph = SupergraphState::new(&schema);
    let hash = |query: &str| {
        normalize_operation(&supergraph, &parse_operation(query), None)
            .unwrap()
            .executable_operation()
            .hash()
    };
    let compact = hash("{ me { id ... @defer { name } ... @defer { email } } }");
    let spread_out = hash(
        "
        {
          me {
            id
            ... @defer { name }
            ... @defer {
              email
            }
          }
        }
        ",
    );
    assert_eq!(compact, spread_out);
    // two `@defer`s with the same fields are still two
    assert_ne!(
        hash("{ me { id ... @defer { name } } }"),
        hash("{ me { id ... @defer { name } ... @defer { name } } }")
    );
}
