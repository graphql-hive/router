use criterion::{criterion_group, criterion_main, Criterion};
use graphql_tools::parser::query::Document;
use hive_router::pipeline::parser::is_introspection_query_only;
use hive_router_query_planner::utils::parsing::safe_parse_operation;
use std::hint::black_box;

// Pre-parsed queries for benchmarking
fn setup_queries() -> Vec<(&'static str, Document<'static, String>)> {
    let simple_introspection = safe_parse_operation(
        r#"
            {
              __typename
            }
        "#,
    )
    .expect("Failed to parse simple_introspection");

    let complex_introspection = safe_parse_operation(
        r#"
            {
              __schema {
                types {
                  name
                  kind
                  description
                  fields {
                    name
                    description
                    type {
                      name
                      kind
                    }
                  }
                  interfaces {
                    name
                  }
                }
              }
            }
        "#,
    )
    .expect("Failed to parse complex_introspection");

    let non_introspection = safe_parse_operation(
        r#"
            {
              user {
                id
                name
                email
              }
            }
        "#,
    )
    .expect("Failed to parse non_introspection");

    let mixed_query = safe_parse_operation(
        r#"
            {
              __typename
              user {
                id
                name
              }
            }
        "#,
    )
    .expect("Failed to parse mixed_query");

    let introspection_with_fragments = safe_parse_operation(
        r#"
            query {
              ...SchemaFields
              ...TypeFields
            }
            fragment SchemaFields on Query {
              __schema {
                types {
                  name
                }
              }
            }
            fragment TypeFields on Query {
              __type(name: "Query") {
                name
                fields {
                  name
                }
              }
            }
        "#,
    )
    .expect("Failed to parse introspection_with_fragments");

    let non_introspection_with_fragments = safe_parse_operation(
        r#"
            query {
              ...UserFields
              ...PostFields
            }
            fragment UserFields on Query {
              user {
                id
                name
              }
            }
            fragment PostFields on Query {
              posts {
                title
                content
              }
            }
        "#,
    )
    .expect("Failed to parse non_introspection_with_fragments");

    let self_referencing_fragment = safe_parse_operation(
        r#"
            query {
              ...FragA
            }
            fragment FragA on Query {
              ...FragA
              __typename
            }
        "#,
    )
    .expect("Failed to parse self_referencing_fragment");

    let circular_fragments = safe_parse_operation(
        r#"
            query {
              ...FragA
            }
            fragment FragA on Query {
              ...FragB
              __typename
            }
            fragment FragB on Query {
              ...FragA
              __typename
            }
        "#,
    )
    .expect("Failed to parse circular_fragments");

    let deeply_nested_inline_fragments = safe_parse_operation(
        r#"
            query {
              ... on Query {
                ... on Query {
                  ... on Query {
                    __typename
                    __schema {
                      types {
                        name
                      }
                    }
                  }
                }
              }
            }
        "#,
    )
    .expect("Failed to parse deeply_nested_inline_fragments");

    let large_introspection_query = safe_parse_operation(
        r#"
            {
              __schema {
                types {
                  name
                  kind
                  description
                  possibleTypes {
                    name
                    kind
                  }
                  interfaces {
                    name
                    kind
                  }
                  fields {
                    name
                    description
                    isDeprecated
                    deprecationReason
                    type {
                      name
                      kind
                      ofType {
                        name
                        kind
                      }
                    }
                    args {
                      name
                      description
                      type {
                        name
                        kind
                      }
                      defaultValue
                    }
                  }
                  enumValues {
                    name
                    description
                    isDeprecated
                  }
                  inputFields {
                    name
                    description
                    type {
                      name
                      kind
                    }
                    defaultValue
                  }
                }
                directives {
                  name
                  description
                  locations
                  args {
                    name
                    description
                    type {
                      name
                      kind
                    }
                  }
                }
              }
              __type(name: "Query") {
                name
                kind
                fields {
                  name
                  type {
                    name
                    kind
                  }
                }
              }
            }
        "#,
    )
    .expect("Failed to parse large_introspection_query");

    vec![
        ("simple_introspection", simple_introspection),
        ("complex_introspection", complex_introspection),
        ("non_introspection", non_introspection),
        ("mixed_query", mixed_query),
        ("introspection_with_fragments", introspection_with_fragments),
        (
            "non_introspection_with_fragments",
            non_introspection_with_fragments,
        ),
        ("self_referencing_fragment", self_referencing_fragment),
        ("circular_fragments", circular_fragments),
        (
            "deeply_nested_inline_fragments",
            deeply_nested_inline_fragments,
        ),
        ("large_introspection_query", large_introspection_query),
    ]
}

fn benchmark_introspection_checks(c: &mut Criterion) {
    let queries = setup_queries();

    let mut group = c.benchmark_group("is_introspection_query_only");

    for (name, query) in &queries {
        group.bench_function(*name, |b| {
            b.iter(|| is_introspection_query_only(black_box(query), None))
        });
    }

    group.finish();
}

criterion_group!(benches, benchmark_introspection_checks);
criterion_main!(benches);
