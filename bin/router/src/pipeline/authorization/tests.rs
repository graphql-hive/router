use std::{collections::HashSet, fmt::Display, sync::Arc};

use crate::pipeline::authorization::metadata::AuthorizationMetadata;
use crate::query_planner::{
    ast::normalization::normalize_operation,
    consumer_schema::ConsumerSchema,
    state::supergraph_state::{OperationKind, SupergraphState},
    utils::parsing::parse_schema,
};
use crate::{
    config::HiveRouterConfig,
    executor::{
        execution::client_request_details::JwtRequestDetails,
        introspection::{
            partition::partition_operation,
            schema::{SchemaMetadata, SchemaWithMetadata},
        },
        projection::plan::FieldProjectionPlan,
        response::graphql_error::GraphQLError,
    },
};
use graphql_tools::parser::parse_query;

use crate::pipeline::{
    authorization::{
        apply_authorization_to_operation, collect_required_policies, AuthorizationDecision,
        AuthorizationError,
    },
    normalize::{hash_normalized_operation, GraphQLNormalizationPayload, OperationIdentity},
};

struct SupergraphTestData {
    pub supergraph_state: SupergraphState,
    pub auth_metadata: AuthorizationMetadata,
    pub schema_metadata: SchemaMetadata,
}

impl Display for AuthorizationDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn print_errors(errors: &[AuthorizationError]) -> Vec<String> {
            errors
                .iter()
                .map(GraphQLError::from)
                .map(|e| {
                    format!(
                        "{} @ {}",
                        e.message,
                        e.extensions.affected_path.as_deref().unwrap_or_default()
                    )
                })
                .collect()
        }

        match self {
            AuthorizationDecision::NoChange => write!(f, "[NoChange]"),
            AuthorizationDecision::Modified {
                new_operation_definition,
                errors,
                ..
            } => {
                write!(
                    f,
                    "[Modified]\nOperation: {}\nErrors:    {:?}",
                    if new_operation_definition.selection_set.is_empty() {
                        "<empty>".to_string()
                    } else {
                        new_operation_definition.to_string()
                    },
                    print_errors(errors)
                )
            }
            AuthorizationDecision::Reject { errors } => {
                write!(f, "[Reject]\n\nErrors: {:?}", print_errors(errors))
            }
        }
    }
}

impl SupergraphTestData {
    fn decide(&self, scopes: Option<Vec<&str>>, operation: &'static str) -> AuthorizationDecision {
        self.decide_with_policies(scopes, &[], operation)
    }

    fn decide_with_policies(
        &self,
        scopes: Option<Vec<&str>>,
        granted_policies: &[&str],
        operation: &'static str,
    ) -> AuthorizationDecision {
        let payload = self.normalize(operation);

        let jwt = if let Some(scopes) = scopes {
            JwtRequestDetails::Authenticated {
                token: "asd".into(),
                prefix: None,
                claims: Default::default(),
                scopes: Some(scopes.iter().map(|s| s.to_string()).collect()),
            }
        } else {
            JwtRequestDetails::Unauthenticated
        };

        // TODO: Fix this
        let granted_policies: HashSet<String> =
            granted_policies.iter().map(|s| s.to_string()).collect();

        apply_authorization_to_operation(
            &payload,
            &self.auth_metadata,
            &self.schema_metadata,
            &Default::default(),
            &jwt,
            &granted_policies,
            true,
            false,
        )
        .expect("test schema/operation should only reference fields declared in the schema")
    }

    /// Returns the policies the operation requires, sorted for stable assertions.
    fn required_policies(&self, operation: &'static str) -> Vec<String> {
        let payload = self.normalize(operation);

        let mut policies: Vec<String> = collect_required_policies(
            &HiveRouterConfig::default(),
            &payload,
            &self.auth_metadata,
            &self.schema_metadata,
            &Default::default(),
        )
        .expect("test schema/operation should only reference fields declared in the schema")
        .into_iter()
        .collect();

        policies.sort();
        policies
    }

    fn normalize(&self, operation: &'static str) -> GraphQLNormalizationPayload {
        let parsed_query = parse_query(operation).unwrap();
        let doc = normalize_operation(&self.supergraph_state, &parsed_query, None).unwrap();
        let operation = doc.operation;
        let operation_kind = operation
            .operation_kind
            .clone()
            .unwrap_or(OperationKind::Query);
        let (root_type_name, projection_plan) =
            FieldProjectionPlan::from_operation(&operation, &self.schema_metadata);
        let root_type_name = root_type_name.to_string();
        let partitioned_operation = partition_operation(operation);
        let operation_for_plan = Arc::new(partitioned_operation.downstream_operation);
        let operation_for_introspection =
            partitioned_operation.introspection_operation.map(Arc::new);

        let hashes =
            hash_normalized_operation(&operation_for_plan, operation_for_introspection.as_deref());

        GraphQLNormalizationPayload {
            root_type_name,
            operation_kind,
            projection_plan: Arc::new(projection_plan),
            operation_for_plan,
            operation_for_plan_hash: hashes.operation_for_plan_hash,
            operation_for_introspection,
            operation_for_introspection_hash: hashes.operation_for_introspection_hash,
            normalized_operation_hash: hashes.combined_operation_hash,
            operation_identity: OperationIdentity {
                name: doc.operation_name.clone(),
                operation_type: OperationKind::Query,
                client_document_hash: "".to_string(),
            },
        }
    }
}

fn build_supergraph_data(supergraph_sdl: &str) -> SupergraphTestData {
    let parsed_schema = parse_schema(&build_supergraph_sdl(supergraph_sdl));
    let supergraph_state = SupergraphState::new(&parsed_schema);
    let consumer_schema = ConsumerSchema::new_from_supergraph(&parsed_schema);
    let schema_metadata = consumer_schema.schema_metadata();
    let auth_metadata = AuthorizationMetadata::build(&supergraph_state, &schema_metadata).unwrap();

    SupergraphTestData {
        supergraph_state,
        auth_metadata,
        schema_metadata,
    }
}

static FED: &str = r#"
  schema
    @link(url: "https://specs.apollo.dev/link/v1.0")
    @link(url: "https://specs.apollo.dev/join/v0.3", for: EXECUTION)
    @link(url: "https://specs.apollo.dev/requiresScopes/v0.1", for: SECURITY)
    @link(url: "https://specs.apollo.dev/authenticated/v0.1", for: SECURITY)
    @link(url: "https://specs.apollo.dev/policy/v0.1", for: SECURITY)
  {
      query: Query
      mutation: Mutation
  }
  directive @link(url: String, as: String, for: link__Purpose, import: [link__Import]) repeatable on SCHEMA
  scalar link__Import
  enum link__Purpose { SECURITY EXECUTION }
  scalar federation__Scope
  scalar federation__Policy
  directive @requiresScopes(scopes: [[federation__Scope!]!]!) on OBJECT | FIELD_DEFINITION | INTERFACE | SCALAR | ENUM
  directive @authenticated on OBJECT | FIELD_DEFINITION | INTERFACE | SCALAR | ENUM
  directive @policy(policies: [[federation__Policy!]!]!) on OBJECT | FIELD_DEFINITION | INTERFACE | SCALAR | ENUM
"#;

fn build_supergraph_sdl(sdl: &str) -> String {
    format!("{}\n{}", FED, sdl)
}

#[cfg(test)]
mod field_authorization {
    use super::*;

    static BLOG_SCHEMA: &str = r#"
        type Query {
          posts: [Post!]
          me: User @requiresScopes(scopes: [["profile"]])
          node(id: ID!): Node
        }

        interface Node @requiresScopes(scopes: [["read:user"]]) {
            id: ID!
        }

        type Post implements Node {
          id: ID!
          title: String
          content: String
          author: User
          comments(first: Int = 5): [Comment!]
          internalNotes: SensitiveData
        }

        scalar SensitiveData @requiresScopes(scopes: [["internal", "audit"]])

        type Comment @requiresScopes(scopes: [["read:comment"]]) {
          id: ID!
          content: String
          author: User
        }

        type User implements Node @requiresScopes(scopes: [["read:user"]]) {
          id: ID!
          username: String @requiresScopes(scopes: [["read:username"]])
          email: String
        }
    "#;

    mod removes_unauthorized {
        use super::*;

        #[test]
        fn removes_field_without_required_scope() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let decision = supergraph_data.decide(
                None,
                r#"
                query {
                  posts {
                    title
                  }
                  me {
                    username
                  }
                }
                "#,
            );

            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {posts{title}}
            Errors:    ["Unauthorized field or type @ me"]
            "#);
        }

        #[test]
        fn removes_field_with_alias_when_unauthorized() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let decision = supergraph_data.decide(
                None,
                "
                query {
                  posts {
                   title
                  }
                  my_account: me {
                    username
                  }
                }
              ",
            );

            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {posts{title}}
            Errors:    ["Unauthorized field or type @ my_account"]
            "#);
        }

        #[test]
        fn removes_scalar_field_with_required_scopes() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let decision = supergraph_data.decide(
                None,
                "
                query {
                  posts {
                    title
                    internalNotes
                  }
                }
                ",
            );

            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {posts{title}}
            Errors:    ["Unauthorized field or type @ posts.internalNotes"]
            "#);
        }

        #[test]
        fn removes_array_field_when_item_type_unauthorized() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let decision = supergraph_data.decide(
                None,
                "
                query {
                  posts {
                    title
                    comments {
                      content
                      author {
                        username
                      }
                    }
                  }
                }
              ",
            );

            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {posts{title}}
            Errors:    ["Unauthorized field or type @ posts.comments"]
            "#);
        }
    }
}

#[cfg(test)]
mod type_authorization {
    use super::*;

    mod interfaces {
        use super::*;

        static SECURED_INTERFACE_TYPE_SCHEMA: &str = r#"
            type Query {
                node(id: ID!): Node!
            }

            interface Node @requiresScopes(scopes: [["a", "c"], ["a", "d"], ["b", "c"], ["b", "d"]]) {
                id: ID
            }

            type Book implements Node @requiresScopes(scopes: [["a"], ["b"]]) {
                id: ID
                pages: Int
            }

            type Movie implements Node @requiresScopes(scopes: [["c"], ["d"]]) {
                id: ID
                minutes: Int
            }
        "#;

        #[test]
        fn removes_interface_field_without_scopes() {
            let supergraph_data = build_supergraph_data(SECURED_INTERFACE_TYPE_SCHEMA);
            let query = "
              query($id: ID!) {
                node(id: $id) {
                  id
                }
              }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ node"]
            "#);
        }

        #[test]
        fn removes_interface_field_with_partial_scopes() {
            let supergraph_data = build_supergraph_data(SECURED_INTERFACE_TYPE_SCHEMA);
            let query = "
              query($id: ID!) {
                node(id: $id) {
                  id
                }
              }
            ";

            let decision = supergraph_data.decide(Some(vec!["a"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ node"]
            "#);
        }

        #[test]
        fn allows_interface_field_with_required_scopes() {
            let supergraph_data = build_supergraph_data(SECURED_INTERFACE_TYPE_SCHEMA);
            let query = "
              query($id: ID!) {
                node(id: $id) {
                  id
                }
              }
            ";

            let decision = supergraph_data.decide(Some(vec!["a", "c"]), query);
            insta::assert_snapshot!(decision, @r#"
              [NoChange]
            "#);
        }

        #[test]
        fn disallows_typename_on_unauthorized_interface() {
            let supergraph_data = build_supergraph_data(SECURED_INTERFACE_TYPE_SCHEMA);
            let query = r#"
              query($id: ID!) {
                node(id: $id) {
                  __typename
                }
              }
            "#;

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ node"]
            "#);
        }

        #[test]
        fn removes_implementing_types_without_combined_scopes() {
            let supergraph_data = build_supergraph_data(SECURED_INTERFACE_TYPE_SCHEMA);
            let query = "
              query($id: ID!) {
                node(id: $id) {
                  ... on Movie {
                    id
                  }
                  ... on Book {
                    id
                  }
                }
              }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ node"]
            "#);
        }

        #[test]
        fn removes_implementing_types_with_partial_combined_scopes() {
            let supergraph_data = build_supergraph_data(SECURED_INTERFACE_TYPE_SCHEMA);
            let query = "
              query($id: ID!) {
                node(id: $id) {
                  ... on Movie {
                    id
                  }
                  ... on Book {
                    id
                  }
                }
              }
            ";

            let decision = supergraph_data.decide(Some(vec!["a"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ node"]
            "#);

            let decision = supergraph_data.decide(Some(vec!["a", "b"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ node"]
            "#);
        }

        #[test]
        fn allows_implementing_types_with_required_combined_scopes() {
            let supergraph_data = build_supergraph_data(SECURED_INTERFACE_TYPE_SCHEMA);
            let query = "
              query($id: ID!) {
                node(id: $id) {
                  ... on Movie {
                    id
                  }
                  ... on Book {
                    id
                  }
                }
              }
            ";

            let decision = supergraph_data.decide(Some(vec!["a", "c"]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }

        static SECURED_INTERFACE_TYPE_FIELD_SCHEMA: &str = r#"
            type Query {
                media: Media!
            }

            interface Media {
                id: ID @requiresScopes(scopes: [["a", "b", "c", "d"]])
                title: String @requiresScopes(scopes: [["title"]])
                score: Int
            }

            type Book implements Media {
                id: ID @requiresScopes(scopes: [["a", "b"]])
                title: String @requiresScopes(scopes: [["title"]])
                score: Int
                pages: Int
            }

            type Movie implements I {
                id: ID @requiresScopes(scopes: [["c", "d"]])
                title: String
                score: Int
                minutes: Int
            }
        "#;

        #[test]
        fn field_scopes_override_interface_scopes_on_interface() {
            let supergraph_data = build_supergraph_data(SECURED_INTERFACE_TYPE_FIELD_SCHEMA);
            let query = "
              query {
                media {
                  id
                  score
                }
              }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {media{score}}
            Errors:    ["Unauthorized field or type @ media.id"]
            "#);
        }

        #[test]
        fn field_scopes_override_interface_scopes_on_implementing_types() {
            let supergraph_data = build_supergraph_data(SECURED_INTERFACE_TYPE_FIELD_SCHEMA);
            let query = "
              query {
                media {
                  ... on Book {
                    id
                    score
                  }
                  ... on Movie {
                    id
                    score
                  }
                }
              }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {media{...on Book{score} ...on Movie{score}}}
            Errors:    ["Unauthorized field or type @ media.id", "Unauthorized field or type @ media.id"]
            "#);
        }

        #[test]
        fn removes_all_fields_when_multiple_unauthorized() {
            let supergraph_data = build_supergraph_data(SECURED_INTERFACE_TYPE_FIELD_SCHEMA);
            let query = "
              query {
                media {
                  id
                  title
                }
              }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ media.id", "Unauthorized field or type @ media.title"]
            "#);
        }

        static SCHEMA_FOR_INTERFACE_TYPENAME: &str = r#"
          type Query {
            post(id: ID!): Post
          }

          interface Post @requiresScopes(scopes: [["b"]]) {
            id: ID!
            title: String! @requiresScopes(scopes: [["c"]])
          }

          type PublicBlog implements Post {
            id: ID!
            title: String!
          }

          type PrivateBlog implements Post @requiresScopes(scopes: [["b"]]) {
            id: ID!
            title: String! @requiresScopes(scopes: [["c"]])
            publishAt: String
          }
        "#;

        #[test]
        fn removes_interface_typename_in_fragment_without_scopes() {
            let supergraph_data = build_supergraph_data(SCHEMA_FOR_INTERFACE_TYPENAME);
            let query = r#"
              query {
                  post(id: "1") {
                    ... on PublicBlog {
                      __typename
                      title
                    }
                  }
                }
           "#;

            let decision = supergraph_data.decide(Some(vec!["profile"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ post"]
            "#);
        }

        #[test]
        fn removes_interface_typename_without_scopes() {
            let supergraph_data = build_supergraph_data(SCHEMA_FOR_INTERFACE_TYPENAME);
            let query = r#"
              query {
                  post(id: "1") {
                    __typename
                    ... on PublicBlog {
                      __typename
                      title
                    }
                  }
                }
           "#;

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ post"]
            "#);
        }

        #[test]
        fn removes_interface_field_without_field_scopes() {
            let supergraph_data = build_supergraph_data(SCHEMA_FOR_INTERFACE_TYPENAME);
            let query = r#"
              query {
                  post(id: "1") {
                    title
                  }
                }
           "#;

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ post"]
            "#);
        }

        #[test]
        fn removes_interface_field_with_type_scope_but_not_field_scope() {
            let supergraph_data = build_supergraph_data(SCHEMA_FOR_INTERFACE_TYPENAME);
            let query = r#"
              query {
                  post(id: "1") {
                    title
                  }
                }
           "#;

            let decision = supergraph_data.decide(Some(vec!["b"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ post.title"]
            "#);
        }

        #[test]
        fn allows_interface_field_with_all_required_scopes() {
            let supergraph_data = build_supergraph_data(SCHEMA_FOR_INTERFACE_TYPENAME);
            let query = r#"
              query {
                  post(id: "1") {
                    title
                  }
                }
           "#;

            let decision = supergraph_data.decide(Some(vec!["b", "c"]), query);
            insta::assert_snapshot!(decision, @r#"
              [NoChange]
            "#);
        }
    }

    mod unions {
        use super::*;

        static UNION_SCHEMA: &str = r#"
          type Query {
              media: Media!
          }

          union Media = Book | Movie

          type Book @requiresScopes(scopes: [["a", "b"]]) {
            title: String
          }

          type Movie @requiresScopes(scopes: [["c", "d"]]) {
            title: String
          }
       "#;

        /// Resolving __typename is allowed since it does not reveal any data.
        ///
        /// In case it's being queried by itself, we do not implement any logic to protect it.
        #[test]
        fn allows_bare_typename_on_a_union_field_regardless_of_member_scopes() {
            let supergraph_data = build_supergraph_data(UNION_SCHEMA);
            let query = "
              query {
                media {
                  __typename
                }
              }
           ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @"[NoChange]");
        }

        /// Each fragment (and concrete type) is authorized independently against its own member's
        /// scopes
        #[test]
        fn removes_all_unauthorized_union_member_fragments() {
            let supergraph_data = build_supergraph_data(UNION_SCHEMA);
            let query = "
              query {
                media {
                  ... on Book {
                    title
                  }
                  ... on Movie {
                    title
                  }
                }
              }
           ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ media", "Unauthorized field or type @ media"]
            "#);
        }

        /// This test ensures that we apply scope validation correctly on unions.
        /// Since we know the concrete type statically, we should be able to apply
        /// member-specific scopes without combining them with the union's own scopes.
        ///
        /// Previously, the authz logic was merging the union member's scopes into one big
        /// rule. But that's wrong since it gates all variations of the union.
        ///
        /// In the schema here, both rules were applied, so in order to access `media`, a token needs to ahve
        /// ALL `a` + `b` + `c` + `d` scopes.
        #[test]
        fn union_members_are_authorized_independently_of_each_other() {
            let supergraph_data = build_supergraph_data(
                r#"
                type Query {
                  media: Media!
                }

                union Media = Book | Movie

                type Book @requiresScopes(scopes: [["a", "b"]]) {
                  author: String
                }

                type Movie @requiresScopes(scopes: [["c", "d"]]) {
                  director: String
                }
                "#,
            );
            let query = "
              query {
                media {
                  ... on Book {
                    author
                  }
                  ... on Movie {
                    director
                  }
                }
              }
           ";

            let decision = supergraph_data.decide(Some(vec!["a", "b"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {media{...on Book{author}}}
            Errors:    ["Unauthorized field or type @ media"]
            "#);

            let decision = supergraph_data.decide(Some(vec!["c", "d"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {media{...on Movie{director}}}
            Errors:    ["Unauthorized field or type @ media"]
            "#);
        }

        /// `Book` and `Movie` both have a `title` field with the same response key.
        ///
        /// Previously, having access to only one of them ended up with an empty output,
        /// because the accessible type's field was also wiped out: rejected paths were
        /// tracked purely by flattened response key (`media.title`).
        #[test]
        fn shared_field_name_across_union_members_is_authorized_independently() {
            let supergraph_data = build_supergraph_data(UNION_SCHEMA);
            let query = "
              query {
                media {
                  ... on Book {
                    title
                  }
                  ... on Movie {
                    title
                  }
                }
              }
           ";

            let decision = supergraph_data.decide(Some(vec!["a", "b"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {media{...on Book{title}}}
            Errors:    ["Unauthorized field or type @ media"]
            "#);
        }

        #[test]
        fn allows_union_field_with_all_member_scopes() {
            let supergraph_data = build_supergraph_data(UNION_SCHEMA);
            let query = "
              query {
                media {
                  ... on Book {
                    title
                  }
                  ... on Movie {
                    title
                  }
                }
              }
           ";

            let decision = supergraph_data.decide(Some(vec!["a", "b", "c", "d"]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }
    }
}

#[cfg(test)]
mod fragments {
    use super::*;

    static BLOG_SCHEMA: &str = r#"
        type Query {
          posts: [Post!]
          me: User @requiresScopes(scopes: [["profile"]])
          node(id: ID!): Node
        }

        interface Node @requiresScopes(scopes: [["read:user"]]) {
            id: ID!
        }

        type Post implements Node {
          id: ID!
          title: String
          content: String
          author: User
          comments(first: Int = 5): [Comment!]
        }

        type Comment @requiresScopes(scopes: [["read:comment"]]) {
          id: ID!
          content: String
          author: User
        }

        type User implements Node @requiresScopes(scopes: [["read:user"]]) {
          id: ID!
          username: String @requiresScopes(scopes: [["read:username"]])
          email: String
        }
    "#;

    mod inline_fragments {
        use super::*;

        #[test]
        fn removes_inline_fragment_on_unauthorized_type() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let query = r#"
              query {
                posts {
                  title
                }
                node(id: "id") {
                  id
                  ... on User {
                    uid: id
                    username
                  }
                }
              }
            "#;

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {posts{title}}
            Errors:    ["Unauthorized field or type @ node"]
            "#);
        }

        #[test]
        fn removes_unauthorized_field_inside_inline_fragment() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let query = r#"
              query {
                posts {
                  title
                }
                node(id: "id") {
                  id
                  ... on User {
                    uid: id
                    username
                  }
                }
              }
            "#;

            let decision = supergraph_data.decide(Some(vec!["read:user"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {posts{title} node(id: "id"){id ...on User{uid: id}}}
            Errors:    ["Unauthorized field or type @ node.username"]
            "#);
        }

        #[test]
        fn allows_inline_fragment_with_all_required_scopes() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let query = r#"
              query {
                posts {
                  title
                }
                node(id: "id") {
                  id
                  ... on User {
                    uid: id
                    username
                  }
                }
              }
            "#;

            let decision = supergraph_data.decide(Some(vec!["read:user", "read:username"]), query);
            insta::assert_snapshot!(decision, @r#"
              [NoChange]
            "#);
        }
    }

    mod named_fragments {
        use super::*;

        #[test]
        fn removes_unauthorized_fields_from_named_fragment() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let query = "
            query {
                posts {
                    title
                    ...PostWithComments
                }
            }

            fragment PostWithComments on Post {
                comments {
                    content
                }
            }
          ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {posts{title}}
            Errors:    ["Unauthorized field or type @ posts.comments"]
            "#);
        }

        #[test]
        fn removes_entire_named_fragment_when_type_unauthorized() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let query = r#"
            query {
                posts {
                    title
                }
                node(id: "id") {
                    id
                    ...UserFragment
                }
            }

            fragment UserFragment on User {
                uid: id
                username
            }
          "#;

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {posts{title}}
            Errors:    ["Unauthorized field or type @ node"]
            "#);
        }

        #[test]
        fn allows_named_fragment_with_type_scope_removes_unauthorized_field() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let query = r#"
            query {
                posts {
                    title
                }
                node(id: "id") {
                    id
                    ...UserFragment
                }
            }

            fragment UserFragment on User {
                uid: id
                username
            }
          "#;

            let decision = supergraph_data.decide(Some(vec!["read:user"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {posts{title} node(id: "id"){id ...on User{uid: id}}}
            Errors:    ["Unauthorized field or type @ node.username"]
            "#);
        }

        #[test]
        fn allows_named_fragment_with_all_required_scopes() {
            let supergraph_data = build_supergraph_data(BLOG_SCHEMA);
            let query = r#"
            query {
                posts {
                    title
                }
                node(id: "id") {
                    id
                    ...UserFragment
                }
            }

            fragment UserFragment on User {
                uid: id
                username
            }
          "#;

            let decision = supergraph_data.decide(Some(vec!["read:user", "read:username"]), query);
            insta::assert_snapshot!(decision, @r#"
              [NoChange]
            "#);
        }
    }
}

#[cfg(test)]
mod variable_cleanup {
    use super::*;

    static VARIABLE_SCHEMA: &str = r#"
        type Query {
            version: String
            node(id: ID!): Node!
        }

        interface Node @requiresScopes(scopes: [["a", "c"], ["a", "d"], ["b", "c"], ["b", "d"]]) {
            id: ID
        }

        type Book implements Node @requiresScopes(scopes: [["a"], ["b"]]) {
            id: ID
            pages: Int
        }

        type Movie implements Node @requiresScopes(scopes: [["c"], ["d"]]) {
            id: ID
            minutes: Int
        }
    "#;

    #[test]
    fn removes_unused_variable_when_field_removed() {
        let supergraph_data = build_supergraph_data(VARIABLE_SCHEMA);
        let query = r#"
          query($id: ID!) {
            version
            node(id: $id) {
              __typename
            }
          }
        "#;

        let decision = supergraph_data.decide(None, query);
        insta::assert_snapshot!(decision, @r#"
        [Modified]
        Operation: {version}
        Errors:    ["Unauthorized field or type @ node"]
        "#);
    }
}

#[cfg(test)]
mod mutations {
    use super::*;

    static MUTATION_SCHEMA: &str = r#"
        type Query {
            posts: [Post!]
            me: User
        }

        type Mutation @requiresScopes(scopes: [["user:write"]]) {
            createPost(title: String!, content: String!): Post @requiresScopes(scopes: [["post:write"]])
            updatePost(id: ID!, title: String): Post @requiresScopes(scopes: [["post:write"]])
            deletePost(id: ID!): Boolean
            addComment(postId: ID!, content: String!): Comment
            publishPost(id: ID!): Post @requiresScopes(scopes: [["post:publish"]])
        }

        type Post {
            id: ID!
            title: String
            content: String
            author: User
        }

        type Comment {
            id: ID!
            content: String
        }

        type User {
            id: ID!
            username: String
        }
    "#;

    mod removes_unauthorized {
        use super::*;

        #[test]
        fn removes_entire_mutation_without_type_scope() {
            let supergraph_data = build_supergraph_data(MUTATION_SCHEMA);
            let query = r#"
                mutation {
                    createPost(title: "Hello", content: "World") {
                        id
                        title
                    }
                }
            "#;

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ createPost"]
            "#);
        }

        #[test]
        fn removes_mutation_field_without_field_scope() {
            let supergraph_data = build_supergraph_data(MUTATION_SCHEMA);
            let query = r#"
                mutation {
                    createPost(title: "Hello", content: "World") {
                        id
                        title
                    }
                }
            "#;

            let decision = supergraph_data.decide(Some(vec!["user:write"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ createPost"]
            "#);
        }

        #[test]
        fn removes_specific_mutation_field_keeps_others() {
            let supergraph_data = build_supergraph_data(MUTATION_SCHEMA);
            let query = r#"
                mutation {
                    deletePost(id: "1")
                    createPost(title: "Hello", content: "World") {
                        id
                    }
                }
            "#;

            let decision = supergraph_data.decide(Some(vec!["user:write"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: mutation{deletePost(id: "1")}
            Errors:    ["Unauthorized field or type @ createPost"]
            "#);
        }

        #[test]
        fn removes_multiple_unauthorized_mutation_fields() {
            let supergraph_data = build_supergraph_data(MUTATION_SCHEMA);
            let query = r#"
                mutation {
                    createPost(title: "Hello", content: "World") {
                        id
                    }
                    publishPost(id: "1") {
                        id
                    }
                    deletePost(id: "2")
                }
            "#;

            let decision = supergraph_data.decide(Some(vec!["user:write"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: mutation{deletePost(id: "2")}
            Errors:    ["Unauthorized field or type @ createPost", "Unauthorized field or type @ publishPost"]
            "#);
        }
    }

    mod allows_with_scopes {
        use super::*;

        #[test]
        fn allows_mutation_with_type_and_field_scopes() {
            let supergraph_data = build_supergraph_data(MUTATION_SCHEMA);
            let query = r#"
                mutation {
                    createPost(title: "Hello", content: "World") {
                        id
                        title
                    }
                }
            "#;

            let decision = supergraph_data.decide(Some(vec!["user:write", "post:write"]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }

        #[test]
        fn allows_mutation_field_without_additional_scope() {
            let supergraph_data = build_supergraph_data(MUTATION_SCHEMA);
            let query = r#"
                mutation {
                    deletePost(id: "1")
                    addComment(postId: "1", content: "Nice!") {
                        id
                    }
                }
            "#;

            let decision = supergraph_data.decide(Some(vec!["user:write"]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }

        #[test]
        fn allows_multiple_mutations_with_different_scopes() {
            let supergraph_data = build_supergraph_data(MUTATION_SCHEMA);
            let query = r#"
                mutation {
                    createPost(title: "Hello", content: "World") {
                        id
                    }
                    publishPost(id: "1") {
                        id
                    }
                }
            "#;

            let decision = supergraph_data.decide(
                Some(vec!["user:write", "post:write", "post:publish"]),
                query,
            );
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }
    }

    mod mixed_authorization {
        use super::*;

        #[test]
        fn allows_some_mutations_removes_others() {
            let supergraph_data = build_supergraph_data(MUTATION_SCHEMA);
            let query = r#"
                mutation {
                    createPost(title: "Hello", content: "World") {
                        id
                        title
                    }
                    deletePost(id: "2")
                    publishPost(id: "3") {
                        id
                    }
                }
            "#;

            let decision = supergraph_data.decide(Some(vec!["user:write", "post:write"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: mutation{createPost(content: "World", title: "Hello"){id title} deletePost(id: "2")}
            Errors:    ["Unauthorized field or type @ publishPost"]
            "#);
        }
    }

    mod with_variables {
        use super::*;

        #[test]
        fn removes_unused_variables_in_mutation() {
            let supergraph_data = build_supergraph_data(MUTATION_SCHEMA);
            let query = r#"
                mutation($title: String!, $content: String!, $id: ID!) {
                    createPost(title: $title, content: $content) {
                        id
                        title
                    }
                    deletePost(id: $id)
                }
            "#;

            let decision = supergraph_data.decide(Some(vec!["user:write"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: mutation($id:ID!){deletePost(id: $id)}
            Errors:    ["Unauthorized field or type @ createPost"]
            "#);
        }

        #[test]
        fn keeps_all_variables_when_mutations_authorized() {
            let supergraph_data = build_supergraph_data(MUTATION_SCHEMA);
            let query = r#"
                mutation($title: String!, $content: String!) {
                    createPost(title: $title, content: $content) {
                        id
                        title
                    }
                }
            "#;

            let decision = supergraph_data.decide(Some(vec!["user:write", "post:write"]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }
    }

    mod return_type_authorization {
        use super::*;

        static MUTATION_WITH_SECURED_RETURN_SCHEMA: &str = r#"
            type Query {
                posts: [Post!]
            }

            type Mutation {
                createPost(title: String!, content: String!): Post
                createUser(username: String!): User
            }

            type Post {
                id: ID!
                title: String
            }

            type User @requiresScopes(scopes: [["read:user"]]) {
                id: ID!
                username: String
            }
        "#;

        #[test]
        fn removes_mutation_when_return_type_unauthorized() {
            let supergraph_data = build_supergraph_data(MUTATION_WITH_SECURED_RETURN_SCHEMA);
            let query = r#"
                mutation {
                    createUser(username: "john") {
                        id
                        username
                    }
                }
            "#;

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ createUser"]
            "#);
        }

        #[test]
        fn allows_mutation_when_return_type_authorized() {
            let supergraph_data = build_supergraph_data(MUTATION_WITH_SECURED_RETURN_SCHEMA);
            let query = r#"
                mutation {
                    createUser(username: "john") {
                        id
                        username
                    }
                }
            "#;

            let decision = supergraph_data.decide(Some(vec!["read:user"]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }

        #[test]
        fn allows_authorized_mutation_removes_unauthorized_return_fields() {
            let supergraph_data = build_supergraph_data(MUTATION_WITH_SECURED_RETURN_SCHEMA);
            let query = r#"
                mutation {
                    createPost(title: "Hello", content: "World") {
                        id
                        title
                    }
                    createUser(username: "john") {
                        id
                        username
                    }
                }
            "#;

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: mutation{createPost(content: "World", title: "Hello"){id title}}
            Errors:    ["Unauthorized field or type @ createUser"]
            "#);
        }
    }
}

#[cfg(test)]
mod authenticated_directive {
    use super::*;

    static AUTHENTICATED_SCHEMA: &str = r#"
        type Query {
            publicPosts: [Post!]
            me: User @authenticated
            profile: Profile @authenticated
        }

        type Mutation {
            createPost(title: String!): Post
            updateProfile(name: String!): Profile
        }

        type Post {
            id: ID!
            title: String
            author: User
        }

        type User @authenticated {
            id: ID!
            username: String
            email: String
        }

        type Profile {
            id: ID!
            name: String
            bio: String
        }
    "#;

    mod field_level {
        use super::*;

        #[test]
        fn removes_authenticated_field_when_unauthenticated() {
            let supergraph_data = build_supergraph_data(AUTHENTICATED_SCHEMA);
            let query = "
                query {
                    publicPosts {
                        id
                        title
                    }
                    me {
                        id
                        username
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {publicPosts{id title}}
            Errors:    ["Unauthorized field or type @ me"]
            "#);
        }

        #[test]
        fn allows_authenticated_field_when_authenticated() {
            let supergraph_data = build_supergraph_data(AUTHENTICATED_SCHEMA);
            let query = "
                query {
                    publicPosts {
                        id
                    }
                    me {
                        id
                        username
                    }
                }
            ";

            // Empty scopes array means authenticated but no scopes
            let decision = supergraph_data.decide(Some(vec![]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }

        #[test]
        fn removes_authenticated_field_with_authenticated_return_type() {
            let supergraph_data = build_supergraph_data(AUTHENTICATED_SCHEMA);
            let query = "
                query {
                    profile {
                        id
                        name
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ profile"]
            "#);
        }
    }

    mod type_level {
        use super::*;

        #[test]
        fn removes_field_when_return_type_requires_authentication() {
            let supergraph_data = build_supergraph_data(AUTHENTICATED_SCHEMA);
            let query = "
                query {
                    publicPosts {
                        id
                        title
                        author {
                            id
                            username
                        }
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {publicPosts{id title}}
            Errors:    ["Unauthorized field or type @ publicPosts.author"]
            "#);
        }

        #[test]
        fn allows_authenticated_type_when_authenticated() {
            let supergraph_data = build_supergraph_data(AUTHENTICATED_SCHEMA);
            let query = "
                query {
                    publicPosts {
                        id
                        author {
                            id
                            username
                        }
                    }
                }
            ";

            let decision = supergraph_data.decide(Some(vec![]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }
    }

    mod mutation_type_level {
        use super::*;

        static MUTATION_TYPE_AUTHENTICATED_SCHEMA: &str = r#"
            type Query {
                publicPosts: [Post!]
            }

            type Mutation @authenticated {
                createPost(title: String!): Post
                updateProfile(name: String!): Profile
            }

            type Post {
                id: ID!
                title: String
            }

            type Profile {
                id: ID!
                name: String
            }
        "#;

        #[test]
        fn removes_entire_mutation_type_when_unauthenticated() {
            let supergraph_data = build_supergraph_data(MUTATION_TYPE_AUTHENTICATED_SCHEMA);
            let query = "
                mutation {
                    createPost(title: \"Hello\") {
                        id
                        title
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ createPost"]
            "#);
        }

        #[test]
        fn allows_mutation_type_when_authenticated() {
            let supergraph_data = build_supergraph_data(MUTATION_TYPE_AUTHENTICATED_SCHEMA);
            let query = "
                mutation {
                    createPost(title: \"Hello\") {
                        id
                        title
                    }
                }
            ";

            let decision = supergraph_data.decide(Some(vec![]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }

        #[test]
        fn removes_all_mutation_fields_when_mutation_type_unauthenticated() {
            let supergraph_data = build_supergraph_data(MUTATION_TYPE_AUTHENTICATED_SCHEMA);
            let query = "
                mutation {
                    createPost(title: \"Hello\") {
                        id
                    }
                    updateProfile(name: \"John\") {
                        id
                        name
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ createPost", "Unauthorized field or type @ updateProfile"]
            "#);
        }
    }

    mod query_type_level {
        use super::*;

        static QUERY_TYPE_AUTHENTICATED_SCHEMA: &str = r#"
            type Query @authenticated {
                posts: [Post!]
                me: User
            }

            type Post {
                id: ID!
                title: String
            }

            type User {
                id: ID!
                username: String
            }
        "#;

        #[test]
        fn removes_entire_query_type_when_unauthenticated() {
            let supergraph_data = build_supergraph_data(QUERY_TYPE_AUTHENTICATED_SCHEMA);
            let query = "
                query {
                    posts {
                        id
                        title
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ posts"]
            "#);
        }

        #[test]
        fn allows_query_type_when_authenticated() {
            let supergraph_data = build_supergraph_data(QUERY_TYPE_AUTHENTICATED_SCHEMA);
            let query = "
                query {
                    posts {
                        id
                        title
                    }
                }
            ";

            let decision = supergraph_data.decide(Some(vec![]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }

        #[test]
        fn removes_all_query_fields_when_query_type_unauthenticated() {
            let supergraph_data = build_supergraph_data(QUERY_TYPE_AUTHENTICATED_SCHEMA);
            let query = "
                query {
                    posts {
                        id
                    }
                    me {
                        id
                        username
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ posts", "Unauthorized field or type @ me"]
            "#);
        }
    }

    mod non_nullable_root_fields {
        use super::*;

        static NON_NULLABLE_QUERY_SCHEMA: &str = r#"
            type Query @authenticated {
                user: User!
                posts: [Post!]
            }

            type User {
                id: ID!
                username: String
            }

            type Post {
                id: ID!
                title: String
            }
        "#;

        static NON_NULLABLE_MUTATION_SCHEMA: &str = r#"
            type Query {
                posts: [Post!]
            }

            type Mutation @authenticated {
                createUser(name: String!): User!
                deletePost(id: ID!): Boolean
            }

            type User {
                id: ID!
                username: String
            }

            type Post {
                id: ID!
                title: String
            }
        "#;

        static MIXED_NULLABILITY_SCHEMA: &str = r#"
            type Query @authenticated {
                requiredUser: User!
                optionalUser: User
                requiredPosts: [Post!]!
                optionalPosts: [Post!]
            }

            type User {
                id: ID!
                username: String
            }

            type Post {
                id: ID!
                title: String
            }
        "#;

        #[test]
        fn marks_non_nullable_query_field_correctly() {
            let supergraph_data = build_supergraph_data(NON_NULLABLE_QUERY_SCHEMA);
            let query = "
                query {
                    user {
                        id
                        username
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ user"]
            "#);
        }

        #[test]
        fn marks_non_nullable_mutation_field_correctly() {
            let supergraph_data = build_supergraph_data(NON_NULLABLE_MUTATION_SCHEMA);
            let mutation = "
                mutation {
                    createUser(name: \"Alice\") {
                        id
                        username
                    }
                }
            ";

            let decision = supergraph_data.decide(None, mutation);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ createUser"]
            "#);
        }

        #[test]
        fn handles_mixed_nullability_in_query() {
            let supergraph_data = build_supergraph_data(MIXED_NULLABILITY_SCHEMA);
            let query = "
                query {
                    requiredUser {
                        id
                    }
                    optionalUser {
                        id
                    }
                    requiredPosts {
                        id
                    }
                    optionalPosts {
                        id
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            // All fields should be removed, and has_non_null_unauthorized should be true
            // due to requiredUser and requiredPosts being non-nullable
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ requiredUser", "Unauthorized field or type @ optionalUser", "Unauthorized field or type @ requiredPosts", "Unauthorized field or type @ optionalPosts"]
            "#);
        }

        #[test]
        fn allows_non_nullable_field_when_authenticated() {
            let supergraph_data = build_supergraph_data(NON_NULLABLE_QUERY_SCHEMA);
            let query = "
                query {
                    user {
                        id
                        username
                    }
                }
            ";

            let decision = supergraph_data.decide(Some(vec![]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }

        #[test]
        fn handles_multiple_non_nullable_fields() {
            let supergraph_data = build_supergraph_data(NON_NULLABLE_QUERY_SCHEMA);
            let query = "
                query {
                    user {
                        id
                    }
                    posts {
                        id
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            // Both fields should be removed, user being non-nullable triggers the flag
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ user", "Unauthorized field or type @ posts"]
            "#);
        }

        #[test]
        fn nullable_mutation_field_when_type_unauthorized() {
            let supergraph_data = build_supergraph_data(NON_NULLABLE_MUTATION_SCHEMA);
            let mutation = "
                mutation {
                    deletePost(id: \"123\")
                }
            ";

            let decision = supergraph_data.decide(None, mutation);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ deletePost"]
            "#);
        }

        #[test]
        fn field_level_non_nullable_triggers_bubbling() {
            // Test that non-nullable fields with field-level auth (not type-level)
            // also correctly trigger null bubbling
            let schema = r#"
                type Query {
                    publicData: String
                    privateUser: User! @authenticated
                    optionalUser: User @authenticated
                }

                type User {
                    id: ID!
                    name: String
                }
            "#;

            let supergraph_data = build_supergraph_data(schema);
            let query = "
                query {
                    publicData
                    privateUser {
                        id
                        name
                    }
                    optionalUser {
                        id
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            // privateUser is non-nullable and unauthorized - should be marked as UnauthorizedNonNullable
            // optionalUser is nullable and unauthorized - should be marked as UnauthorizedNullable
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {publicData}
            Errors:    ["Unauthorized field or type @ privateUser", "Unauthorized field or type @ optionalUser"]
            "#);
        }

        #[test]
        fn bubbling_stops_at_nearest_nullable_ancestor() {
            let schema = r#"
                type Query {
                    me: User
                    publicData: String
                }

                type User {
                    id: ID!
                    posts: Posts
                }

                type Posts {
                    title: String! @authenticated
                }
            "#;

            let supergraph_data = build_supergraph_data(schema);
            let query = "
                query {
                    me {
                        id
                        posts {
                            title
                        }
                    }
                    publicData
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {me{id} publicData}
            Errors:    ["Unauthorized field or type @ me.posts.title"]
            "#);
        }
    }

    mod mutation_and_query_type_combined {
        use super::*;

        static BOTH_TYPES_AUTHENTICATED_SCHEMA: &str = r#"
            type Query @authenticated {
                publicPosts: [Post!]
                me: User
            }

            type Mutation @authenticated {
                createPost(title: String!): Post
            }

            type Post {
                id: ID!
                title: String
            }

            type User {
                id: ID!
                username: String
            }
        "#;

        #[test]
        fn removes_query_and_mutation_when_unauthenticated() {
            let supergraph_data = build_supergraph_data(BOTH_TYPES_AUTHENTICATED_SCHEMA);
            let query = "
                query {
                    publicPosts {
                        id
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ publicPosts"]
            "#);

            let mutation = "
                mutation {
                    createPost(title: \"Hello\") {
                        id
                    }
                }
            ";

            let decision = supergraph_data.decide(None, mutation);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["Unauthorized field or type @ createPost"]
            "#);
        }

        #[test]
        fn allows_both_query_and_mutation_when_authenticated() {
            let supergraph_data = build_supergraph_data(BOTH_TYPES_AUTHENTICATED_SCHEMA);
            let query = "
                query {
                    publicPosts {
                        id
                    }
                }
            ";

            let decision = supergraph_data.decide(Some(vec![]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);

            let mutation = "
                mutation {
                    createPost(title: \"Hello\") {
                        id
                    }
                }
            ";

            let decision = supergraph_data.decide(Some(vec![]), mutation);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }
    }

    mod combined_with_scopes {
        use super::*;

        static COMBINED_SCHEMA: &str = r#"
            type Query {
                publicPosts: [Post!]
                adminPanel: AdminPanel @authenticated @requiresScopes(scopes: [["admin"]])
            }

            type Post {
                id: ID!
                title: String
                content: String @requiresScopes(scopes: [["read:content"]])
            }

            type AdminPanel @authenticated {
                users: [User!] @requiresScopes(scopes: [["read:users"]])
                settings: Settings
            }

            type User {
                id: ID!
                username: String
            }

            type Settings {
                theme: String
            }
        "#;

        #[test]
        fn removes_field_without_authentication() {
            let supergraph_data = build_supergraph_data(COMBINED_SCHEMA);
            let query = "
                query {
                    publicPosts {
                        id
                        title
                    }
                    adminPanel {
                        settings {
                            theme
                        }
                    }
                }
            ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {publicPosts{id title}}
            Errors:    ["Unauthorized field or type @ adminPanel"]
            "#);
        }

        #[test]
        fn removes_field_with_authentication_but_without_scope() {
            let supergraph_data = build_supergraph_data(COMBINED_SCHEMA);
            let query = "
                query {
                    publicPosts {
                        id
                    }
                    adminPanel {
                        settings {
                            theme
                        }
                    }
                }
            ";

            let decision = supergraph_data.decide(Some(vec![]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {publicPosts{id}}
            Errors:    ["Unauthorized field or type @ adminPanel"]
            "#);
        }

        #[test]
        fn allows_field_with_authentication_and_scope() {
            let supergraph_data = build_supergraph_data(COMBINED_SCHEMA);
            let query = "
                query {
                    publicPosts {
                        id
                    }
                    adminPanel {
                        settings {
                            theme
                        }
                    }
                }
            ";

            let decision = supergraph_data.decide(Some(vec!["admin"]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }

        #[test]
        fn removes_nested_field_without_scope_but_keeps_authenticated_parent() {
            let supergraph_data = build_supergraph_data(COMBINED_SCHEMA);
            let query = "
                query {
                    adminPanel {
                        users {
                            id
                        }
                        settings {
                            theme
                        }
                    }
                }
            ";

            let decision = supergraph_data.decide(Some(vec!["admin"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {adminPanel{settings{theme}}}
            Errors:    ["Unauthorized field or type @ adminPanel.users"]
            "#);
        }

        #[test]
        fn allows_nested_field_with_all_required_scopes() {
            let supergraph_data = build_supergraph_data(COMBINED_SCHEMA);
            let query = "
                query {
                    adminPanel {
                        users {
                            id
                            username
                        }
                        settings {
                            theme
                        }
                    }
                }
            ";

            let decision = supergraph_data.decide(Some(vec!["admin", "read:users"]), query);
            insta::assert_snapshot!(decision, @r#"
            [NoChange]
            "#);
        }
    }

    mod with_variables {
        use super::*;

        #[test]
        fn removes_unused_variables_when_authenticated_field_removed() {
            let supergraph_data = build_supergraph_data(AUTHENTICATED_SCHEMA);
            let query = r#"
                query($name: String!) {
                    publicPosts {
                        id
                    }
                    profile {
                        name
                    }
                }
            "#;

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {publicPosts{id}}
            Errors:    ["Unauthorized field or type @ profile"]
            "#);
        }
    }
}

#[cfg(test)]
mod policy_directive {
    use super::*;

    static POLICY_SCHEMA: &str = r#"
        type Query {
          publicPosts: [Post!]
          profile: Profile @policy(policies: [["read_profile"]])
          billing: Billing
          audit: AuditLog @policy(policies: [["admin"], ["auditor", "compliance"]])
          secret: String @authenticated @policy(policies: [["read_secret"]])
        }

        type Post {
          id: ID!
          title: String
        }

        type Profile {
          name: String
          email: String @policy(policies: [["read_email"]])
        }

        type Billing @policy(policies: [["read_billing"]]) {
          plan: String
        }

        type AuditLog {
          entries: [String!]
        }
    "#;

    #[test]
    fn denies_policy_protected_field_when_nothing_is_granted() {
        let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

        let decision = supergraph_data.decide_with_policies(
            None,
            &[],
            "{ publicPosts { id } profile { name } }",
        );
        insta::assert_snapshot!(decision, @r#"
        [Modified]
        Operation: {publicPosts{id}}
        Errors:    ["Unauthorized field or type @ profile"]
        "#);
    }

    #[test]
    fn allows_policy_protected_field_when_policy_is_granted() {
        let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

        let decision = supergraph_data.decide_with_policies(
            None,
            &["read_profile"],
            "{ publicPosts { id } profile { name } }",
        );
        insta::assert_snapshot!(decision, @"[NoChange]");
    }

    #[test]
    fn denies_nested_policy_protected_field_while_keeping_its_parent() {
        let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

        let decision = supergraph_data.decide_with_policies(
            None,
            &["read_profile"],
            "{ profile { name email } }",
        );
        insta::assert_snapshot!(decision, @r#"
        [Modified]
        Operation: {profile{name}}
        Errors:    ["Unauthorized field or type @ profile.email"]
        "#);
    }

    #[test]
    fn denies_field_whose_output_type_carries_a_policy() {
        let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

        let decision = supergraph_data.decide_with_policies(None, &[], "{ billing { plan } }");
        insta::assert_snapshot!(decision, @r#"
        [Modified]
        Operation: <empty>
        Errors:    ["Unauthorized field or type @ billing"]
        "#);
    }

    #[test]
    fn allows_field_whose_output_type_carries_a_granted_policy() {
        let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

        let decision =
            supergraph_data.decide_with_policies(None, &["read_billing"], "{ billing { plan } }");
        insta::assert_snapshot!(decision, @"[NoChange]");
    }

    /// `[["admin"], ["auditor", "compliance"]]` is an OR of ANDs: either "admin"
    /// alone, or both "auditor" and "compliance".
    #[test]
    fn treats_policy_groups_as_or_of_ands() {
        let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

        let granted_admin =
            supergraph_data.decide_with_policies(None, &["admin"], "{ audit { entries } }");
        insta::assert_snapshot!(granted_admin, @"[NoChange]");

        let granted_both = supergraph_data.decide_with_policies(
            None,
            &["auditor", "compliance"],
            "{ audit { entries } }",
        );
        insta::assert_snapshot!(granted_both, @"[NoChange]");

        let granted_partial =
            supergraph_data.decide_with_policies(None, &["auditor"], "{ audit { entries } }");
        insta::assert_snapshot!(granted_partial, @r#"
        [Modified]
        Operation: <empty>
        Errors:    ["Unauthorized field or type @ audit"]
        "#);
    }

    /// `@policy` is independent of `@authenticated`: both must be satisfied when
    /// they sit on the same field.
    #[test]
    fn requires_both_authentication_and_policy_when_combined() {
        let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

        let policy_only =
            supergraph_data.decide_with_policies(None, &["read_secret"], "{ secret }");
        insta::assert_snapshot!(policy_only, @r#"
        [Modified]
        Operation: <empty>
        Errors:    ["Unauthorized field or type @ secret"]
        "#);

        let authenticated_only =
            supergraph_data.decide_with_policies(Some(vec![]), &[], "{ secret }");
        insta::assert_snapshot!(authenticated_only, @r#"
        [Modified]
        Operation: <empty>
        Errors:    ["Unauthorized field or type @ secret"]
        "#);

        let both =
            supergraph_data.decide_with_policies(Some(vec![]), &["read_secret"], "{ secret }");
        insta::assert_snapshot!(both, @"[NoChange]");
    }

    /// Unknown policies are ignored rather than granting anything.
    #[test]
    fn ignores_policies_that_are_not_declared_in_the_schema() {
        let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

        let decision =
            supergraph_data.decide_with_policies(None, &["not_a_policy"], "{ profile { name } }");
        insta::assert_snapshot!(decision, @r#"
        [Modified]
        Operation: <empty>
        Errors:    ["Unauthorized field or type @ profile"]
        "#);
    }

    mod required_policies {
        use super::*;

        #[test]
        fn collects_nothing_for_an_operation_without_policies() {
            let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

            assert!(supergraph_data
                .required_policies("{ publicPosts { id } }")
                .is_empty());
        }

        #[test]
        fn collects_policies_from_fields_and_output_types() {
            let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

            insta::assert_debug_snapshot!(
                supergraph_data.required_policies("{ profile { name email } billing { plan } }"),
                @r#"
            [
                "read_billing",
                "read_email",
                "read_profile",
            ]
            "#
            );
        }

        /// Every policy of an OR group is reported, the decision of which
        /// combination is enough belongs to enforcement.
        #[test]
        fn collects_every_policy_of_an_or_group() {
            let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

            insta::assert_debug_snapshot!(
                supergraph_data.required_policies("{ audit { entries } }"),
                @r#"
            [
                "admin",
                "auditor",
                "compliance",
            ]
            "#
            );
        }

        /// Fields excluded by `@skip`/`@include` must not drag their policies in.
        #[test]
        fn ignores_fields_excluded_by_skip() {
            let supergraph_data = build_supergraph_data(POLICY_SCHEMA);

            assert!(supergraph_data
                .required_policies("{ publicPosts { id } profile @skip(if: true) { name } }")
                .is_empty());
        }
    }

    mod abstract_types {
        use super::*;

        static POLICY_UNION_SCHEMA: &str = r#"
            type Query {
              media: Media!
            }

            union Media = Book | Movie

            type Book @policy(policies: [["read_book"]]) {
              author: String
            }

            type Movie @policy(policies: [["read_movie"]]) {
              director: String
            }
        "#;

        #[test]
        fn union_members_with_policy_are_authorized_independently() {
            let supergraph_data = build_supergraph_data(POLICY_UNION_SCHEMA);
            let query = "
              { media { ... on Book { author } ... on Movie { director } } }
            ";

            let decision = supergraph_data.decide_with_policies(None, &["read_book"], query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {media{...on Book{author}}}
            Errors:    ["Unauthorized field or type @ media"]
            "#);

            let decision = supergraph_data.decide_with_policies(None, &["read_movie"], query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {media{...on Movie{director}}}
            Errors:    ["Unauthorized field or type @ media"]
            "#);

            let decision =
                supergraph_data.decide_with_policies(None, &["read_book", "read_movie"], query);
            insta::assert_snapshot!(decision, @"[NoChange]");
        }

        /// `Book` and `Movie` both expose `title` under this response key. This is
        /// the exact scenario that used to collapse both fragments into one
        /// null-bubbling trie entry and wipe out an already-authorized member.
        #[test]
        fn shared_field_name_across_union_members_with_policy_is_authorized_independently() {
            let supergraph_data = build_supergraph_data(
                r#"
                type Query {
                  media: Media!
                }

                union Media = Book | Movie

                type Book @policy(policies: [["read_book"]]) {
                  title: String
                }

                type Movie @policy(policies: [["read_movie"]]) {
                  title: String
                }
                "#,
            );
            let query = "
              { media { ... on Book { title } ... on Movie { title } } }
            ";

            let decision = supergraph_data.decide_with_policies(None, &["read_book"], query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {media{...on Book{title}}}
            Errors:    ["Unauthorized field or type @ media"]
            "#);
        }

        static POLICY_INTERFACE_SCHEMA: &str = r#"
            type Query {
              itf: Node!
            }

            interface Node {
              id: ID!
            }

            type Post implements Node @policy(policies: [["read_post"]]) {
              id: ID!
              title: String
            }

            type Comment implements Node @policy(policies: [["read_comment"]]) {
              id: ID!
              body: String
            }
        "#;

        /// Same independence check, through an interface's concrete-type fragments
        /// instead of a union's members.
        #[test]
        fn interface_implementors_with_policy_are_authorized_independently() {
            let supergraph_data = build_supergraph_data(POLICY_INTERFACE_SCHEMA);
            let query = "
              { itf { ... on Post { title } ... on Comment { body } } }
            ";

            let decision = supergraph_data.decide_with_policies(None, &["read_post"], query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: {itf{...on Post{title}}}
            Errors:    ["Unauthorized field or type @ itf"]
            "#);

            let decision =
                supergraph_data.decide_with_policies(None, &["read_post", "read_comment"], query);
            insta::assert_snapshot!(decision, @"[NoChange]");
        }
    }
}
