use criterion::{criterion_group, criterion_main, Criterion};
use hive_router::pipeline::{
    authorization::{apply_authorization_to_operation, AuthorizationMetadata},
    coerce_variables::CoerceVariablesPayload,
    normalize::{GraphQLNormalizationPayload, OperationIdentity},
};
use hive_router_plan_executor::{
    execution::client_request_details::JwtRequestDetails,
    introspection::{
        partition::partition_operation,
        schema::{SchemaMetadata, SchemaWithMetadata},
    },
    projection::plan::FieldProjectionPlan,
};
use hive_router_query_planner::{
    ast::normalization::normalize_operation,
    planner::Planner,
    state::supergraph_state::SupergraphState,
    utils::parsing::{parse_schema, safe_parse_operation},
};
use std::{hint::black_box, sync::Arc};

struct BenchEnv<'a> {
    normalized_payload: &'a GraphQLNormalizationPayload,
    auth_metadata: &'a AuthorizationMetadata,
    schema_metadata: &'a SchemaMetadata,
    variable_payload: &'a CoerceVariablesPayload,
}

fn authorization_benchmark(c: &mut Criterion) {
    let supergraph_sdl = get_supergraph_sdl();
    let parsed_supergraph_sdl = parse_schema(supergraph_sdl);
    let supergraph_state = SupergraphState::new(&parsed_supergraph_sdl);
    let planner = Planner::new_from_supergraph(&parsed_supergraph_sdl).unwrap();
    let metadata = planner.consumer_schema.schema_metadata();
    let authorization = AuthorizationMetadata::build(&planner.supergraph, &metadata).unwrap();

    fn prepare<'a>(
        supergraph: &SupergraphState,
        metadata: &SchemaMetadata,
        // authorization: &AuthorizationMetadata,
        query: &str,
    ) -> GraphQLNormalizationPayload {
        let parsed = safe_parse_operation(query).unwrap();
        let normalized = normalize_operation(supergraph, &parsed, None).unwrap();
        let (root_type_name, projection_plan) =
            FieldProjectionPlan::from_operation(&normalized.operation, &metadata);
        let partitioned_operation = partition_operation(normalized.operation);

        GraphQLNormalizationPayload {
            root_type_name,
            projection_plan: Arc::new(projection_plan),
            operation_for_plan: Arc::new(partitioned_operation.downstream_operation),
            operation_for_introspection: partitioned_operation
                .introspection_operation
                .map(Arc::new),
            operation_indentity: OperationIdentity {
                name: None,
                operation_type: "query",
                client_document_hash: "".to_string(),
            },
        }
    }

    let bubble_up_payload = prepare(
        &supergraph_state,
        &metadata,
        "query { topProducts(first: 1) { name price } }",
    );
    let bubble_up_variable_payload = CoerceVariablesPayload {
        variables_map: None,
    };
    let bubble_up = BenchEnv {
        normalized_payload: &bubble_up_payload,
        auth_metadata: &authorization,
        schema_metadata: &metadata,
        variable_payload: &bubble_up_variable_payload,
    };

    c.bench_function("bubble", |b| {
        b.iter(|| {
            let jwt_req_details = JwtRequestDetails::Unauthenticated;

            black_box(apply_authorization_to_operation(
                bubble_up.normalized_payload,
                bubble_up.auth_metadata,
                bubble_up.schema_metadata,
                bubble_up.variable_payload,
                &jwt_req_details,
                false,
            ));
        })
    });

    let complex_payload = prepare(
        &supergraph_state,
        &metadata,
        "query { topProducts { name shippingEstimate reviews { body } } me { name birthday } }",
    );
    let complex_variable_payload = CoerceVariablesPayload {
        variables_map: None,
    };
    let complex = BenchEnv {
        normalized_payload: &complex_payload,
        auth_metadata: &authorization,
        schema_metadata: &metadata,
        variable_payload: &complex_variable_payload,
    };

    c.bench_function("complex unauth", |b| {
        b.iter(|| {
            let jwt_req_details = JwtRequestDetails::Unauthenticated;

            black_box(apply_authorization_to_operation(
                complex.normalized_payload,
                complex.auth_metadata,
                complex.schema_metadata,
                complex.variable_payload,
                &jwt_req_details,
                false,
            ));
        })
    });

    let complex_partially_payload = prepare(
        &supergraph_state,
        &metadata,
        "query { topProducts { name shippingEstimate reviews { body } } me { name birthday } }",
    );
    let complex_partially_variable_payload = CoerceVariablesPayload {
        variables_map: None,
    };
    let complex_partially = BenchEnv {
        normalized_payload: &complex_partially_payload,
        auth_metadata: &authorization,
        schema_metadata: &metadata,
        variable_payload: &complex_partially_variable_payload,
    };

    c.bench_function("complex partially unauth", |b| {
        b.iter(|| {
            let jwt_req_details = JwtRequestDetails::Authenticated {
                token: "123".into(),
                prefix: Some("Bearer".into()),
                claims: sonic_rs::Value::new(),
                scopes: Some(vec!["read:shipping".to_string()]),
            };

            black_box(apply_authorization_to_operation(
                complex_partially.normalized_payload,
                complex_partially.auth_metadata,
                complex_partially.schema_metadata,
                complex_partially.variable_payload,
                &jwt_req_details,
                false,
            ));
        })
    });

    // Large query, mostly authorized (2 denied out of ~25 fields)
    let large_mostly_auth_payload = prepare(
        &supergraph_state,
        &metadata,
        r#"query {
            topProducts(first: 20) {
                upc
                weight
                name
                price
                inStock
                shippingEstimate
                notes
                internal
                reviews {
                    id
                    body
                    author {
                        id
                        name
                        username
                        birthday
                    }
                }
            }
            users {
                id
                name
                username
                birthday
                reviews {
                    id
                    body
                }
            }
            me {
                id
                name
                username
                birthday
                reviews {
                    id
                    body
                }
            }
        }"#,
    );
    let large_mostly_auth_var_payload = CoerceVariablesPayload {
        variables_map: None,
    };
    let large_mostly_auth = BenchEnv {
        normalized_payload: &large_mostly_auth_payload,
        auth_metadata: &authorization,
        schema_metadata: &metadata,
        variable_payload: &large_mostly_auth_var_payload,
    };

    c.bench_function("large mostly authorized", |b| {
        b.iter(|| {
            let jwt_req_details = JwtRequestDetails::Authenticated {
                token: "123".into(),
                prefix: Some("Bearer".into()),
                claims: sonic_rs::Value::new(),
                scopes: Some(vec![
                    "read:price".to_string(),
                    "read:shipping".to_string(),
                    "read:birthday".to_string(),
                ]),
            };

            black_box(apply_authorization_to_operation(
                large_mostly_auth.normalized_payload,
                large_mostly_auth.auth_metadata,
                large_mostly_auth.schema_metadata,
                large_mostly_auth.variable_payload,
                &jwt_req_details,
                false,
            ));
        })
    });

    // Large query, partially denied (6 denied out of ~25 fields)
    let large_partially_denied_payload = prepare(
        &supergraph_state,
        &metadata,
        r#"query {
            topProducts(first: 20) {
                upc
                weight
                name
                price
                inStock
                shippingEstimate
                notes
                internal
                reviews {
                    id
                    body
                    author {
                        id
                        name
                        username
                        birthday
                    }
                }
            }
            me {
                id
                name
                username
                birthday
                reviews {
                    id
                    body
                }
            }
        }"#,
    );
    let large_partially_denied_var_payload = CoerceVariablesPayload {
        variables_map: None,
    };
    let large_partially_denied = BenchEnv {
        normalized_payload: &large_partially_denied_payload,
        auth_metadata: &authorization,
        schema_metadata: &metadata,
        variable_payload: &large_partially_denied_var_payload,
    };

    c.bench_function("large partially denied", |b| {
        b.iter(|| {
            // Authenticated but with no scopes
            let jwt_req_details = JwtRequestDetails::Authenticated {
                token: "123".into(),
                prefix: Some("Bearer".into()),
                claims: sonic_rs::Value::new(),
                scopes: Some(vec![]),
            };

            black_box(apply_authorization_to_operation(
                large_partially_denied.normalized_payload,
                large_partially_denied.auth_metadata,
                large_partially_denied.schema_metadata,
                large_partially_denied.variable_payload,
                &jwt_req_details,
                false,
            ));
        })
    });

    // Deep nested query with denied fields
    let deep_nested_payload = prepare(
        &supergraph_state,
        &metadata,
        r#"query {
            topProducts(first: 10) {
                upc
                name
                price
                weight
                inStock
                shippingEstimate
                notes
                internal
                reviews {
                    id
                    body
                    author {
                        id
                        name
                        username
                        birthday
                        reviews {
                            id
                            body
                            product {
                                upc
                                name
                                price
                                weight
                                notes
                                internal
                                reviews {
                                    id
                                    body
                                }
                            }
                        }
                    }
                    product {
                        upc
                        name
                        price
                        shippingEstimate
                    }
                }
            }
        }"#,
    );
    let deep_nested_var_payload = CoerceVariablesPayload {
        variables_map: None,
    };
    let deep_nested = BenchEnv {
        normalized_payload: &deep_nested_payload,
        auth_metadata: &authorization,
        schema_metadata: &metadata,
        variable_payload: &deep_nested_var_payload,
    };

    c.bench_function("deep nested scattered", |b| {
        b.iter(|| {
            let jwt_req_details = JwtRequestDetails::Authenticated {
                token: "123".into(),
                prefix: Some("Bearer".into()),
                claims: sonic_rs::Value::new(),
                scopes: Some(vec!["read:price".to_string(), "read:shipping".to_string()]),
            };

            black_box(apply_authorization_to_operation(
                deep_nested.normalized_payload,
                deep_nested.auth_metadata,
                deep_nested.schema_metadata,
                deep_nested.variable_payload,
                &jwt_req_details,
                false,
            ));
        })
    });

    // Large query, fully unauthenticated (many denied fields)
    let large_unauth_payload = prepare(
        &supergraph_state,
        &metadata,
        r#"query {
            topProducts(first: 20) {
                upc
                weight
                name
                price
                inStock
                shippingEstimate
                notes
                internal
                reviews {
                    id
                    body
                    author {
                        id
                        name
                        username
                    }
                }
            }
            me {
                id
                name
            }
        }"#,
    );
    let large_unauth_var_payload = CoerceVariablesPayload {
        variables_map: None,
    };
    let large_unauth = BenchEnv {
        normalized_payload: &large_unauth_payload,
        auth_metadata: &authorization,
        schema_metadata: &metadata,
        variable_payload: &large_unauth_var_payload,
    };

    c.bench_function("large unauthenticated", |b| {
        b.iter(|| {
            let jwt_req_details = JwtRequestDetails::Unauthenticated;

            black_box(apply_authorization_to_operation(
                large_unauth.normalized_payload,
                large_unauth.auth_metadata,
                large_unauth.schema_metadata,
                large_unauth.variable_payload,
                &jwt_req_details,
                false,
            ));
        })
    });

    let interface_auth_inline_unauth_payload = prepare(
        &supergraph_state,
        &metadata,
        r#"query {
            me {
                name
                socialAccounts {
                    url
                    handle
                    ... on TwitterAccount {
                        followers
                    }
                    ... on GitHubAccount {
                        repoCount
                    }
                }
            }
        }"#,
    );
    let interface_auth_inline_unauth_var_payload = CoerceVariablesPayload {
        variables_map: None,
    };
    let interface_auth_inline_unauth = BenchEnv {
        normalized_payload: &interface_auth_inline_unauth_payload,
        auth_metadata: &authorization,
        schema_metadata: &metadata,
        variable_payload: &interface_auth_inline_unauth_var_payload,
    };

    c.bench_function("interface auth inline fragments unauth", |b| {
        b.iter(|| {
            let jwt_req_details = JwtRequestDetails::Unauthenticated;

            black_box(apply_authorization_to_operation(
                interface_auth_inline_unauth.normalized_payload,
                interface_auth_inline_unauth.auth_metadata,
                interface_auth_inline_unauth.schema_metadata,
                interface_auth_inline_unauth.variable_payload,
                &jwt_req_details,
                false,
            ));
        })
    });

    // Same query but authenticated
    let interface_auth_inline_auth_payload = prepare(
        &supergraph_state,
        &metadata,
        r#"query {
            me {
                name
                socialAccounts {
                    url
                    handle
                    ... on TwitterAccount {
                        followers
                    }
                    ... on GitHubAccount {
                        repoCount
                    }
                }
            }
        }"#,
    );
    let interface_auth_inline_auth_var_payload = CoerceVariablesPayload {
        variables_map: None,
    };
    let interface_auth_inline_auth = BenchEnv {
        normalized_payload: &interface_auth_inline_auth_payload,
        auth_metadata: &authorization,
        schema_metadata: &metadata,
        variable_payload: &interface_auth_inline_auth_var_payload,
    };

    c.bench_function("interface auth inline fragments auth", |b| {
        b.iter(|| {
            let jwt_req_details = JwtRequestDetails::Authenticated {
                token: "123".into(),
                prefix: Some("Bearer".into()),
                claims: sonic_rs::Value::new(),
                scopes: Some(vec![]),
            };

            black_box(apply_authorization_to_operation(
                interface_auth_inline_auth.normalized_payload,
                interface_auth_inline_auth.auth_metadata,
                interface_auth_inline_auth.schema_metadata,
                interface_auth_inline_auth.variable_payload,
                &jwt_req_details,
                false,
            ));
        })
    });
}

criterion_group!(benches, authorization_benchmark);
criterion_main!(benches);

fn get_supergraph_sdl<'a>() -> &'a str {
    r#"
    schema
      @link(url: "https://specs.apollo.dev/link/v1.0")
      @link(url: "https://specs.apollo.dev/join/v0.3", for: EXECUTION)
      @link(url: "https://specs.apollo.dev/requiresScopes/v0.1", for: SECURITY)
      @link(url: "https://specs.apollo.dev/authenticated/v0.1", for: SECURITY) {
      query: Query
    }

    directive @join__enumValue(graph: join__Graph!) repeatable on ENUM_VALUE

    directive @requiresScopes(
      scopes: [[requiresScopes__Scope!]!]!
    ) on FIELD_DEFINITION | OBJECT | INTERFACE | SCALAR | ENUM
    directive @authenticated on FIELD_DEFINITION | OBJECT | INTERFACE | SCALAR | ENUM

    scalar requiresScopes__Scope

    directive @join__field(
      graph: join__Graph
      requires: join__FieldSet
      provides: join__FieldSet
      type: String
      external: Boolean
      override: String
      usedOverridden: Boolean
    ) repeatable on FIELD_DEFINITION | INPUT_FIELD_DEFINITION

    directive @join__graph(name: String!, url: String!) on ENUM_VALUE

    directive @join__implements(
      graph: join__Graph!
      interface: String!
    ) repeatable on OBJECT | INTERFACE

    directive @join__type(
      graph: join__Graph!
      key: join__FieldSet
      extension: Boolean! = false
      resolvable: Boolean! = true
      isInterfaceObject: Boolean! = false
    ) repeatable on OBJECT | INTERFACE | UNION | ENUM | INPUT_OBJECT | SCALAR

    directive @join__unionMember(
      graph: join__Graph!
      member: String!
    ) repeatable on UNION

    directive @link(
      url: String
      as: String
      for: link__Purpose
      import: [link__Import]
    ) repeatable on SCHEMA

    scalar join__FieldSet

    enum join__Graph {
      ACCOUNTS @join__graph(name: "accounts", url: "http://0.0.0.0:4200/accounts")
      INVENTORY
        @join__graph(name: "inventory", url: "http://0.0.0.0:4200/inventory")
      PRODUCTS @join__graph(name: "products", url: "http://0.0.0.0:4200/products")
      REVIEWS @join__graph(name: "reviews", url: "http://0.0.0.0:4200/reviews")
    }

    scalar link__Import

    enum link__Purpose {
      """
      `SECURITY` features provide metadata necessary to securely resolve fields.
      """
      SECURITY

      """
      `EXECUTION` features provide metadata necessary for operation execution.
      """
      EXECUTION
    }

    type Product
      @join__type(graph: INVENTORY, key: "upc")
      @join__type(graph: PRODUCTS, key: "upc")
      @join__type(graph: REVIEWS, key: "upc") {
      upc: String!
      weight: Int
        @join__field(graph: INVENTORY, external: true)
        @join__field(graph: PRODUCTS)
      price: Int!
        @join__field(graph: INVENTORY, external: true)
        @join__field(graph: PRODUCTS)
        @requiresScopes(scopes: [["read:price"]])
      inStock: Boolean @join__field(graph: INVENTORY)
      shippingEstimate: Int
        @join__field(graph: INVENTORY, requires: "price weight")
        @requiresScopes(scopes: [["read:shipping"]])
      name: String @join__field(graph: PRODUCTS)
      reviews: [Review] @join__field(graph: REVIEWS)
      notes: String
        @join__field(graph: PRODUCTS)
        @requiresScopes(scopes: [["read:notes"], ["admin"]])
      internal: String
        @join__field(graph: PRODUCTS)
        @requiresScopes(scopes: [["read:internal", "admin"]])
    }

    type Query
      @join__type(graph: ACCOUNTS)
      @join__type(graph: INVENTORY)
      @join__type(graph: PRODUCTS)
      @join__type(graph: REVIEWS) {
      me: User @join__field(graph: ACCOUNTS) @authenticated
      user(id: ID!): User @join__field(graph: ACCOUNTS)
      users: [User] @join__field(graph: ACCOUNTS)
      topProducts(first: Int = 5): [Product] @join__field(graph: PRODUCTS)
    }

    type Review @join__type(graph: REVIEWS, key: "id") {
      id: ID!
      body: String @authenticated
      product: Product
      author: User @join__field(graph: REVIEWS, provides: "username")
    }

    interface SocialAccount @join__type(graph: ACCOUNTS) @authenticated {
      url: String!
      handle: String!
    }

    type TwitterAccount
      implements SocialAccount
      @join__implements(graph: ACCOUNTS, interface: "SocialAccount")
      @join__type(graph: ACCOUNTS)
      @authenticated {
      url: String!
      handle: String!
      followers: Int!
    }

    type GitHubAccount
      implements SocialAccount
      @join__implements(graph: ACCOUNTS, interface: "SocialAccount")
      @join__type(graph: ACCOUNTS)
      @authenticated {
      url: String!
      handle: String!
      repoCount: Int!
    }

    type User
      @join__type(graph: ACCOUNTS, key: "id")
      @join__type(graph: REVIEWS, key: "id") {
      id: ID!
      name: String @join__field(graph: ACCOUNTS)
      username: String
        @join__field(graph: ACCOUNTS)
        @join__field(graph: REVIEWS, external: true)
      birthday: Int
        @join__field(graph: ACCOUNTS)
        @requiresScopes(scopes: [["read:birthday"]])
      reviews: [Review] @join__field(graph: REVIEWS)
      socialAccounts: [SocialAccount!]! @join__field(graph: ACCOUNTS)
    }

    "#
}
