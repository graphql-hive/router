use std::collections::HashMap;

use super::{project_incremental, DeferPlans, IncrementalFormat};
use crate::executor::introspection::schema::SchemaWithMetadata;
use crate::executor::operation_filter::PathSegment;
use crate::executor::projection::plan::ProjectionPlan;
use crate::executor::response::graphql_error::{
    GraphQLError, GraphQLErrorPath, GraphQLErrorPathSegment,
};
use crate::executor::response::value::Value;
use crate::pipeline::trie::Trie;
use crate::query_planner::ast::normalization::normalize_operation;
use crate::query_planner::consumer_schema::ConsumerSchema;
use crate::query_planner::state::supergraph_state::SupergraphState;
use crate::query_planner::utils::parsing::{parse_operation, parse_schema};

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
  age: Int!
  friends: [User]
}

type Post implements Node
  @join__type(graph: A, key: "id")
  @join__implements(graph: A, interface: "Node") {
  id: ID!
  title: String
  friends: [User]
}
"#;

struct Case {
    query: &'static str,
    data: &'static str,
    errors: Vec<GraphQLError>,
    variables: Option<HashMap<String, sonic_rs::Value>>,
    fail_after: Option<usize>,
}

impl Case {
    fn new(query: &'static str, data: &'static str) -> Self {
        Self {
            query,
            data,
            errors: vec![],
            variables: None,
            fail_after: None,
        }
    }

    /// Writing the part after `parts` later ones fails, see `project_incremental`.
    fn fail_after(mut self, parts: usize) -> Self {
        self.fail_after = Some(parts);
        self
    }

    fn error(mut self, message: &str, path: &[&str]) -> Self {
        let mut error = GraphQLError::from(message);
        error.path = Some(GraphQLErrorPath {
            segments: path
                .iter()
                .map(|s| match s.parse::<usize>() {
                    Ok(index) => GraphQLErrorPathSegment::Index(index),
                    Err(_) => GraphQLErrorPathSegment::String(s.to_string()),
                })
                .collect(),
        });
        self.errors.push(error);
        self
    }

    fn var(mut self, name: &str, value: bool) -> Self {
        self.variables
            .get_or_insert_with(HashMap::new)
            .insert(name.to_string(), sonic_rs::Value::from(value));
        self
    }

    fn run(self, format: IncrementalFormat) -> String {
        self.run_with(format, &[])
    }

    /// `rejected` are the paths authorization took out, from the root, `Fragment` segments as `...Type`.
    fn run_with(self, format: IncrementalFormat, rejected: &[&[&str]]) -> String {
        let supergraph = parse_schema(SCHEMA);
        let schema = ConsumerSchema::new_from_supergraph(&supergraph, true).schema_metadata();
        let state = SupergraphState::new(&supergraph);
        let normalized = normalize_operation(&state, &parse_operation(self.query), None).unwrap();
        let operation = normalized.executable_operation();
        let (root, primary) = ProjectionPlan::from_operation(operation, &schema);
        let plans = std::sync::Arc::new(DeferPlans::build(operation, &schema).expect("has @defer"));
        let paths: Vec<Vec<PathSegment>> = rejected
            .iter()
            .map(|path| {
                path.iter()
                    .map(|segment| match segment.strip_prefix("...") {
                        Some(type_name) => PathSegment::Fragment(type_name),
                        None => PathSegment::Field(segment),
                    })
                    .collect()
            })
            .collect();
        let trie = Trie::from_paths(&paths);
        let primary = std::sync::Arc::new(primary).rewrite(&trie);
        let plans = plans.rewrite(&trie);
        let data_json: sonic_rs::Value = sonic_rs::from_str(self.data).unwrap();
        let data = Value::from(data_json.as_ref());
        let parts = project_incremental(
            &data,
            self.errors,
            &Default::default(),
            root,
            &primary,
            &plans,
            &self.variables,
            1024,
            &schema,
            format,
            self.fail_after,
        )
        .unwrap();
        parts
            .into_iter()
            .map(|part| String::from_utf8(part).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn spec(self) -> String {
        self.run(IncrementalFormat::Spec)
    }

    fn legacy(self) -> String {
        self.run(IncrementalFormat::Legacy)
    }
}

const ME: &str = r#"{"me":{"__typename":"User","id":"1","name":"Ann","email":"a@b.c","age":30}}"#;

#[test]
fn primary_leaves_deferred_fields_out() {
    let case = || Case::new(r#"{ me { id ... @defer(label: "d") { name } } }"#, ME);
    insta::assert_snapshot!(case().spec(), @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"],"label":"d"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Ann"}}],"completed":[{"id":"0"}],"hasNext":false}
    "#);
    insta::assert_snapshot!(case().legacy(), @r#"
    {"data":{"me":{"id":"1"}},"hasNext":true}
    {"incremental":[{"data":{"name":"Ann"},"path":["me"],"label":"d"}],"hasNext":false}
    "#);
}

#[test]
fn if_false_keeps_fields_in_primary() {
    let query = r#"query ($d: Boolean!) { me { id ... @defer(if: $d) { name } } }"#;
    let plans_active = {
        let supergraph = parse_schema(SCHEMA);
        let schema = ConsumerSchema::new_from_supergraph(&supergraph, true).schema_metadata();
        let state = SupergraphState::new(&supergraph);
        let normalized = normalize_operation(&state, &parse_operation(query), None).unwrap();
        let plans = DeferPlans::build(normalized.executable_operation(), &schema).unwrap();
        let mut vars = HashMap::new();
        vars.insert("d".to_string(), sonic_rs::Value::from(false));
        plans.has_active(&Some(vars))
    };
    assert!(!plans_active);
    insta::assert_snapshot!(Case::new(query, ME).var("d", true).spec(), @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Ann"}}],"completed":[{"id":"0"}],"hasNext":false}
    "#);
}

#[test]
fn root_level_defer() {
    insta::assert_snapshot!(
        Case::new(r#"{ ... @defer { me { id } } users { id } }"#, r#"{"me":{"id":"1"},"users":[]}"#).spec(),
        @r#"
    {"data":{"users":[]},"pending":[{"id":"0","path":[]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"me":{"id":"1"}}}],"completed":[{"id":"0"}],"hasNext":false}
    "#
    );
}

#[test]
fn list_items_get_their_own_ids() {
    let case = || {
        Case::new(
            r#"{ users { id ... @defer { name } } }"#,
            r#"{"users":[{"id":"1","name":"Ann"},null,{"id":"3","name":"Bob"}]}"#,
        )
    };
    insta::assert_snapshot!(case().spec(), @r#"
    {"data":{"users":[{"id":"1"},null,{"id":"3"}]},"pending":[{"id":"0","path":["users",0]},{"id":"1","path":["users",2]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Ann"}},{"id":"1","data":{"name":"Bob"}}],"completed":[{"id":"0"},{"id":"1"}],"hasNext":false}
    "#);
    insta::assert_snapshot!(case().legacy(), @r#"
    {"data":{"users":[{"id":"1"},null,{"id":"3"}]},"hasNext":true}
    {"incremental":[{"data":{"name":"Ann"},"path":["users",0]},{"data":{"name":"Bob"},"path":["users",2]}],"hasNext":false}
    "#);
}

#[test]
fn nested_defers_go_out_level_by_level() {
    let case = || {
        Case::new(
            r#"{ me { id ... @defer(label: "outer") { name friends { id ... @defer(label: "inner") { email } } } } }"#,
            r#"{"me":{"id":"1","name":"Ann","friends":[{"id":"2","email":"x@y.z"}]}}"#,
        )
    };
    insta::assert_snapshot!(case().spec(), @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"],"label":"outer"}],"hasNext":true}
    {"pending":[{"id":"1","path":["me","friends",0],"label":"inner"}],"incremental":[{"id":"0","data":{"name":"Ann","friends":[{"id":"2"}]}}],"completed":[{"id":"0"}],"hasNext":true}
    {"incremental":[{"id":"1","data":{"email":"x@y.z"}}],"completed":[{"id":"1"}],"hasNext":false}
    "#);
    insta::assert_snapshot!(case().legacy(), @r#"
    {"data":{"me":{"id":"1"}},"hasNext":true}
    {"incremental":[{"data":{"name":"Ann","friends":[{"id":"2"}]},"path":["me"],"label":"outer"}],"hasNext":true}
    {"incremental":[{"data":{"email":"x@y.z"},"path":["me","friends",0],"label":"inner"}],"hasNext":false}
    "#);
}

#[test]
/// A part that fails to write ends every announced fragment with the error and
/// `hasNext: false`, the ones it was delivering too. `inner` comes up while `outer`'s part is
/// written, so when that one fails it was never announced and is left out.
fn failed_part_ends_the_announced_fragments() {
    let case = |parts| {
        Case::new(
            r#"{ me { id ... @defer(label: "outer") { name friends { id ... @defer(label: "inner") { email } } } } }"#,
            r#"{"me":{"id":"1","name":"Ann","friends":[{"id":"2","email":"x@y.z"}]}}"#,
        )
        .fail_after(parts)
    };
    insta::assert_snapshot!(case(0).spec(), @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"],"label":"outer"}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"failed"}]}],"hasNext":false}
    "#);
    insta::assert_snapshot!(case(1).spec(), @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"],"label":"outer"}],"hasNext":true}
    {"pending":[{"id":"1","path":["me","friends",0],"label":"inner"}],"incremental":[{"id":"0","data":{"name":"Ann","friends":[{"id":"2"}]}}],"completed":[{"id":"0"}],"hasNext":true}
    {"completed":[{"id":"1","errors":[{"message":"failed"}]}],"hasNext":false}
    "#);
    insta::assert_snapshot!(case(1).legacy(), @r#"
    {"data":{"me":{"id":"1"}},"hasNext":true}
    {"incremental":[{"data":{"name":"Ann","friends":[{"id":"2"}]},"path":["me"],"label":"outer"}],"hasNext":true}
    {"incremental":[{"data":null,"path":["me","friends",0],"label":"inner","errors":[{"message":"failed"}]}],"hasNext":false}
    "#);
}

#[test]
fn null_anchor_is_never_announced() {
    insta::assert_snapshot!(
        Case::new(r#"{ me { id ... @defer { name } } }"#, r#"{"me":null}"#).spec(),
        @r#"{"data":{"me":null},"hasNext":false}"#
    );
}

#[test]
fn anchor_nulled_by_propagation_is_never_announced() {
    // `age` is non-null, so `me` comes out null in the primary part
    insta::assert_snapshot!(
        Case::new(
            r#"{ me { age ... @defer { name } } }"#,
            r#"{"me":{"age":null,"name":"Ann"}}"#,
        )
        .error("no age", &["me", "age"])
        .spec(),
        @r#"{"data":{"me":null},"errors":[{"message":"no age","path":["me","age"]}],"hasNext":false}"#
    );
}

#[test]
fn null_in_fragment_stops_at_the_fragment() {
    let case = || {
        Case::new(
            r#"{ me { id ... @defer { name age } } }"#,
            r#"{"me":{"id":"1","name":"Ann","age":null}}"#,
        )
        .error("no age", &["me", "age"])
    };
    insta::assert_snapshot!(case().spec(), @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"no age","path":["me","age"]}]}],"hasNext":false}
    "#);
    insta::assert_snapshot!(case().legacy(), @r#"
    {"data":{"me":{"id":"1"}},"hasNext":true}
    {"incremental":[{"data":null,"path":["me"],"errors":[{"message":"no age","path":["me","age"]}]}],"hasNext":false}
    "#);
}

#[test]
fn errors_go_to_the_part_that_owns_them() {
    insta::assert_snapshot!(
        Case::new(
            r#"{ me { id ... @defer { name } } users { id } }"#,
            r#"{"me":{"id":"1","name":null},"users":null}"#,
        )
        .error("no name", &["me", "name"])
        .error("no users", &["users"])
        .error("no path", &[])
        .spec(),
        @r#"
    {"data":{"me":{"id":"1"},"users":null},"errors":[{"message":"no users","path":["users"]},{"message":"no path","path":[]}],"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":null},"errors":[{"message":"no name","path":["me","name"]}]}],"completed":[{"id":"0"}],"hasNext":false}
    "#
    );
}

#[test]
fn defer_on_interface_is_one_group() {
    insta::assert_snapshot!(
        Case::new(
            r#"{ node { id ... @defer(label: "n") { id ... on User { name } ... on Post { title } } } }"#,
            r#"{"node":{"__typename":"Post","id":"p1","title":"Hello"}}"#,
        )
        .spec(),
        @r#"
    {"data":{"node":{"id":"p1"}},"pending":[{"id":"0","path":["node"],"label":"n"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"title":"Hello"}}],"completed":[{"id":"0"}],"hasNext":false}
    "#
    );
}

#[test]
fn defer_on_a_type_that_does_not_match_is_skipped() {
    insta::assert_snapshot!(
        Case::new(
            r#"{ node { id ... on Post @defer { title } } }"#,
            r#"{"node":{"__typename":"User","id":"u1"}}"#,
        )
        .spec(),
        @r#"{"data":{"node":{"id":"u1"}},"hasNext":false}"#
    );
}

#[test]
fn inactive_parent_moves_child_up() {
    insta::assert_snapshot!(
        Case::new(
            r#"query ($o: Boolean!) { me { id ... @defer(if: $o, label: "outer") { name friends { id ... @defer(label: "inner") { email } } } } }"#,
            r#"{"me":{"id":"1","name":"Ann","friends":[{"id":"2","email":"x@y.z"}]}}"#,
        )
        .var("o", false)
        .spec(),
        @r#"
    {"data":{"me":{"id":"1","name":"Ann","friends":[{"id":"2"}]}},"pending":[{"id":"0","path":["me","friends",0],"label":"inner"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"email":"x@y.z"}}],"completed":[{"id":"0"}],"hasNext":false}
    "#
    );
}

#[test]
fn rejected_fields_are_null_in_deferred_parts_too() {
    // `email` is in the data, like a field the router fetched for itself, but it was rejected
    let case = || {
        Case::new(
            r#"{ me { id ... @defer { name email } } }"#,
            r#"{"me":{"id":"1","name":"Ann","email":"secret"}}"#,
        )
    };
    insta::assert_snapshot!(
        case().run_with(IncrementalFormat::Spec, &[&["me", "...User", "email"]]),
        @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Ann","email":null}}],"completed":[{"id":"0"}],"hasNext":false}
    "#
    );
}

#[test]
fn rejected_fields_are_null_in_deferred_fragments_on_interfaces() {
    insta::assert_snapshot!(
        Case::new(
            r#"{ node { id ... @defer { ... on User { name email } } } }"#,
            r#"{"node":{"__typename":"User","id":"1","name":"Ann","email":"secret"}}"#,
        )
        .run_with(IncrementalFormat::Spec, &[&["node", "...User", "email"]]),
        @r#"
    {"data":{"node":{"id":"1"}},"pending":[{"id":"0","path":["node"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Ann","email":null}}],"completed":[{"id":"0"}],"hasNext":false}
    "#
    );
}

#[test]
fn null_can_not_replace_a_field_that_went_out() {
    // `me` is in the primary part, the null from `age` stops at the root fragment
    insta::assert_snapshot!(
        Case::new(
            "{ me { id ... @defer { name } } ... @defer { me { age } } }",
            r#"{"me":{"id":"1","name":"Ann","age":null}}"#,
        )
        .spec(),
        @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]},{"id":"1","path":[]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Ann"}}],"completed":[{"id":"0"},{"id":"1","errors":[{"message":"Cannot return null for non-nullable field.","path":[]}]}],"hasNext":false}
    "#
    );
}

#[test]
fn fields_that_went_out_are_not_sent_again() {
    insta::assert_snapshot!(
        Case::new(
            r#"{ me { id friends { id } ... @defer { id friends { id name } } } }"#,
            r#"{"me":{"__typename":"User","id":"1","friends":[{"__typename":"User","id":"2","name":"Bo"},{"__typename":"User","id":"3","name":"Cy"}]}}"#,
        )
        .spec(),
        @r#"
    {"data":{"me":{"id":"1","friends":[{"id":"2"},{"id":"3"}]}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"friends":[{"name":"Bo"},{"name":"Cy"}]}}],"completed":[{"id":"0"}],"hasNext":false}
    "#
    );
}

#[test]
fn nested_defer_under_a_field_that_went_out() {
    insta::assert_snapshot!(
        Case::new(
            r#"{ me { id friends { id } ... @defer { friends { ... @defer { name } } } } }"#,
            r#"{"me":{"__typename":"User","id":"1","friends":[{"__typename":"User","id":"2","name":"Bo"}]}}"#,
        )
        .spec(),
        @r#"
    {"data":{"me":{"id":"1","friends":[{"id":"2"}]}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"pending":[{"id":"1","path":["me","friends",0]}],"incremental":[{"id":"0","data":{"friends":[{}]}}],"completed":[{"id":"0"}],"hasNext":true}
    {"incremental":[{"id":"1","data":{"name":"Bo"}}],"completed":[{"id":"1"}],"hasNext":false}
    "#
    );
}

#[test]
fn null_can_not_replace_a_field_a_sibling_fragment_delivers() {
    // `friends` comes from both fragments. The null from `age` would take out the friend the
    // other fragment delivers, so the fragment with `age` fails, whichever comes first
    let data = r#"{"me":{"friends":[{"name":"Bo","age":null}]}}"#;
    insta::assert_snapshot!(
        Case::new(
            r#"{ ... @defer(label: "A") { me { friends { age } } } me { ... @defer(label: "B") { friends { name } } } }"#,
            data,
        )
        .spec(),
        @r#"
    {"data":{"me":{}},"pending":[{"id":"0","path":[],"label":"A"},{"id":"1","path":["me"],"label":"B"}],"hasNext":true}
    {"incremental":[{"id":"1","data":{"friends":[{"name":"Bo"}]}}],"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":[]}]},{"id":"1"}],"hasNext":false}
    "#
    );
    insta::assert_snapshot!(
        Case::new(
            r#"{ me { ... @defer(label: "B") { friends { name } } } ... @defer(label: "A") { me { friends { age } } } }"#,
            data,
        )
        .spec(),
        @r#"
    {"data":{"me":{}},"pending":[{"id":"0","path":["me"],"label":"B"},{"id":"1","path":[],"label":"A"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"friends":[{"name":"Bo"}]}}],"completed":[{"id":"0"},{"id":"1","errors":[{"message":"Cannot return null for non-nullable field.","path":[]}]}],"hasNext":false}
    "#
    );
}

#[test]
fn null_on_a_field_no_other_part_delivers_goes_out() {
    // only fragment A has `friends`, its null takes nothing away
    insta::assert_snapshot!(
        Case::new(
            r#"{ me { id ... @defer(label: "A") { friends { age } } ... @defer(label: "B") { name } } }"#,
            r#"{"me":{"id":"1","name":"Ann","friends":[{"age":null}]}}"#,
        )
        .spec(),
        @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"],"label":"A"},{"id":"1","path":["me"],"label":"B"}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"friends":[null]}},{"id":"1","data":{"name":"Ann"}}],"completed":[{"id":"0"},{"id":"1"}],"hasNext":false}
    "#
    );
}

#[test]
fn null_from_the_data_on_a_shared_field_goes_out() {
    // `friends` is null in the data, the primary part sent the same null
    insta::assert_snapshot!(
        Case::new(
            r#"{ me { friends { id } } ... @defer { me { friends { name } } } }"#,
            r#"{"me":{"friends":null}}"#,
        )
        .spec(),
        @r#"
    {"data":{"me":{"friends":null}},"pending":[{"id":"0","path":[]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"me":{"friends":null}}}],"completed":[{"id":"0"}],"hasNext":false}
    "#
    );
}

#[test]
fn a_field_the_other_part_skipped_is_not_shared() {
    // the primary part has `friends` only with `$f`. Without it, the fragment's null is its own
    let case = |f: bool| {
        Case::new(
            r#"query ($f: Boolean!) { me { id friends @include(if: $f) { id } ... @defer { friends { age } } } }"#,
            r#"{"me":{"id":"1","friends":[{"id":"2","age":null}]}}"#,
        )
        .var("f", f)
        .spec()
    };
    insta::assert_snapshot!(case(false), @r#"
    {"data":{"me":{"id":"1"}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"friends":[null]}}],"completed":[{"id":"0"}],"hasNext":false}
    "#);
    insta::assert_snapshot!(case(true), @r#"
    {"data":{"me":{"id":"1","friends":[{"id":"2"}]}},"pending":[{"id":"0","path":["me"]}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["me"]}]}],"hasNext":false}
    "#);
}

#[test]
fn a_field_the_other_part_has_on_another_type_is_not_shared() {
    // the primary part has `friends` on a `Post` only, the node is a `User`
    let case = |on: &str| {
        let query = format!(
            "{{ node {{ ... on {on} {{ friends {{ id }} }} ... on User @defer {{ name friends {{ age }} }} }} }}"
        );
        Case::new(
            Box::leak(query.into_boxed_str()),
            r#"{"node":{"__typename":"User","name":"Ann","friends":[{"id":"2","age":null}]}}"#,
        )
        .spec()
    };
    insta::assert_snapshot!(case("Post"), @r#"
    {"data":{"node":{}},"pending":[{"id":"0","path":["node"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"name":"Ann","friends":[null]}}],"completed":[{"id":"0"}],"hasNext":false}
    "#);
    insta::assert_snapshot!(case("User"), @r#"
    {"data":{"node":{"friends":[{"id":"2"}]}},"pending":[{"id":"0","path":["node"]}],"hasNext":true}
    {"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["node"]}]}],"hasNext":false}
    "#);
}

#[test]
fn a_field_of_a_sibling_fragment_that_failed_is_not_shared() {
    // fragment B fails on `age`, so nothing of it went out, and A's null takes nothing away
    insta::assert_snapshot!(
        Case::new(
            r#"{ me { ... @defer(label: "B") { age friends { name } } } ... @defer(label: "A") { me { friends { age } } } }"#,
            r#"{"me":{"age":null,"friends":[{"name":"Bo","age":null}]}}"#,
        )
        .spec(),
        @r#"
    {"data":{"me":{}},"pending":[{"id":"0","path":["me"],"label":"B"},{"id":"1","path":[],"label":"A"}],"hasNext":true}
    {"incremental":[{"id":"1","data":{"me":{"friends":[null]}}}],"completed":[{"id":"0","errors":[{"message":"Cannot return null for non-nullable field.","path":["me"]}]},{"id":"1"}],"hasNext":false}
    "#
    );
}

#[test]
fn type_conditions_are_followed_part_by_part() {
    // `friends` is shared on a `User`, but `friends.friends` only comes from the `Post` fragment
    insta::assert_snapshot!(
        Case::new(
            r#"{ node { ... on Post { friends { friends { id } } } ... on User { friends { name } } ... on User @defer { friends { friends { age } } } } }"#,
            r#"{"node":{"__typename":"User","friends":[{"name":"Bo","friends":[{"id":"3","age":null}]}]}}"#,
        )
        .spec(),
        @r#"
    {"data":{"node":{"friends":[{"name":"Bo"}]}},"pending":[{"id":"0","path":["node"]}],"hasNext":true}
    {"incremental":[{"id":"0","data":{"friends":[{"friends":[null]}]}}],"completed":[{"id":"0"}],"hasNext":false}
    "#
    );
}
