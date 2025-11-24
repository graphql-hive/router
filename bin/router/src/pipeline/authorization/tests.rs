use std::{fmt::Display, sync::Arc};

use graphql_parser::parse_query;
use hive_router_plan_executor::{
    execution::client_request_details::JwtRequestDetails,
    introspection::{
        partition::partition_operation,
        schema::{SchemaMetadata, SchemaWithMetadata},
    },
    projection::plan::FieldProjectionPlan,
};
use hive_router_query_planner::{
    ast::normalization::normalize_operation, consumer_schema::ConsumerSchema,
    state::supergraph_state::SupergraphState, utils::parsing::parse_schema,
};

use crate::pipeline::{
    authorization::{
        apply_authorization_to_operation, AuthorizationDecision, AuthorizationMetadata,
    },
    normalize::GraphQLNormalizationPayload,
};

struct SupergraphTestData {
    pub supergraph_state: SupergraphState,
    pub auth_metadata: AuthorizationMetadata,
    pub schema_metadata: SchemaMetadata,
}

impl Display for AuthorizationDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
                    errors.iter().map(|e| e.path.clone()).collect::<Vec<_>>()
                )
            }
            AuthorizationDecision::Reject { errors } => {
                write!(
                    f,
                    "[Reject]\n\nErrors: {:?}",
                    errors.iter().map(|e| e.path.clone()).collect::<Vec<_>>()
                )
            }
        }
    }
}

impl SupergraphTestData {
    fn decide(&self, scopes: Option<Vec<&str>>, operation: &'static str) -> AuthorizationDecision {
        let parsed_query = parse_query(operation).unwrap();
        let doc = normalize_operation(&self.supergraph_state, &parsed_query, None).unwrap();
        let operation = doc.operation;
        let (root_type_name, projection_plan) =
            FieldProjectionPlan::from_operation(&operation, &self.schema_metadata);
        let partitioned_operation = partition_operation(operation);

        let payload = GraphQLNormalizationPayload {
            root_type_name,
            projection_plan: Arc::new(projection_plan),
            operation_for_plan: Arc::new(partitioned_operation.downstream_operation),
            operation_for_introspection: partitioned_operation
                .introspection_operation
                .map(Arc::new),
        };

        let jwt = if let Some(scopes) = scopes {
            JwtRequestDetails::Authenticated {
                token: "asd",
                prefix: None,
                claims: &Default::default(),
                scopes: Some(scopes.iter().map(|s| s.to_string()).collect()),
            }
        } else {
            JwtRequestDetails::Unauthenticated
        };

        apply_authorization_to_operation(
            &payload,
            &self.auth_metadata,
            &self.schema_metadata,
            &Default::default(),
            &jwt,
            false,
        )
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
  {
      query: Query
      mutation: Mutation
  }
  directive @link(url: String, as: String, for: link__Purpose, import: [link__Import]) repeatable on SCHEMA
  scalar link__Import
  enum link__Purpose { SECURITY EXECUTION }
  scalar federation__Scope
  directive @requiresScopes(scopes: [[federation__Scope!]!]!) on OBJECT | FIELD_DEFINITION | INTERFACE | SCALAR | ENUM
  directive @authenticated on OBJECT | FIELD_DEFINITION | INTERFACE | SCALAR | ENUM
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
            Errors:    ["me"]
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
            Errors:    ["my_account"]
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
            Errors:    ["posts.internalNotes"]
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
            Errors:    ["posts.comments"]
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
              Errors:    ["node"]
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
              Errors:    ["node"]
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
              Errors:    ["node"]
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
            Errors:    ["node"]
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
            Errors:    ["node"]
            "#);

            let decision = supergraph_data.decide(Some(vec!["a", "b"]), query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["node"]
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
            Errors:    ["media.id"]
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
            Errors:    ["media.id", "media.id"]
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
            Errors:    ["media.id", "media.title"]
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
            Errors:    ["post"]
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
            Errors:    ["post"]
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
            Errors:    ["post"]
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
            Errors:    ["post.title"]
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

        #[test]
        fn disallows_typename_on_unauthorized_union_members() {
            let supergraph_data = build_supergraph_data(UNION_SCHEMA);
            let query = "
              query {
                media {
                  __typename
                }
              }
           ";

            let decision = supergraph_data.decide(None, query);
            insta::assert_snapshot!(decision, @r#"
            [Modified]
            Operation: <empty>
            Errors:    ["media"]
            "#);
        }

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
            Errors:    ["media"]
            "#);
        }

        #[test]
        fn removes_union_fragments_with_partial_member_scopes() {
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
            Operation: <empty>
            Errors:    ["media"]
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
            Errors:    ["node"]
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
            Errors:    ["node.username"]
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
              Errors:    ["posts.comments"]
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
              Errors:    ["node"]
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
              Errors:    ["node.username"]
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
          Errors:    ["node"]
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
            Errors:    ["createPost"]
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
            Errors:    ["createPost"]
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
            Errors:    ["createPost"]
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
            Errors:    ["createPost", "publishPost"]
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
            Errors:    ["publishPost"]
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
            Errors:    ["createPost"]
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
            Errors:    ["createUser"]
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
            Errors:    ["createUser"]
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
            Errors:    ["me"]
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
            Errors:    ["profile"]
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
            Errors:    ["publicPosts.author"]
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
            Errors:    ["createPost"]
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
            Errors:    ["createPost", "updateProfile"]
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
            Errors:    ["posts"]
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
            Errors:    ["posts", "me"]
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
            Errors:    ["user"]
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
            Errors:    ["createUser"]
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
            Errors:    ["requiredUser", "optionalUser", "requiredPosts", "optionalPosts"]
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
            Errors:    ["user", "posts"]
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
            Errors:    ["deletePost"]
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
            Errors:    ["privateUser", "optionalUser"]
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
            Errors:    ["publicPosts"]
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
            Errors:    ["createPost"]
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
            Errors:    ["adminPanel"]
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
            Errors:    ["adminPanel"]
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
            Errors:    ["adminPanel.users"]
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
            Errors:    ["profile"]
            "#);
        }
    }
}
