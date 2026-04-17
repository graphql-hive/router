use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use hive_router::pipeline::persisted_documents::extract::{
    DocumentIdResolver, DocumentIdResolverInput, HttpRequestContext,
};
use hive_router_config::persisted_documents::PersistedDocumentsConfig;
use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
use std::hint::black_box;

struct PathCase {
    name: &'static str,
    template: &'static str,
    hit_path: &'static str,
    miss_path: &'static str,
}

const GRAPHQL_ENDPOINT: &str = "/graphql";

fn build_resolver(template: &str) -> DocumentIdResolver {
    let manifest_path = std::env::temp_dir().join("persisted-docs-bench.json");
    std::fs::write(&manifest_path, "{}").expect("bench manifest should be writable");

    let raw = format!(
        r#"{{
  "enabled": true,
  "storage": {{
    "type": "file",
    "path": "{}",
    "watch": false
  }},
  "selectors": [
    {{ "type": "url_path_param", "template": "{template}" }}
  ]
}}"#,
        manifest_path.display()
    );

    let config: PersistedDocumentsConfig =
        serde_json::from_str(&raw).expect("bench config should parse");
    DocumentIdResolver::from_config(&config, GRAPHQL_ENDPOINT)
        .expect("resolver config should compile")
}

fn build_query_param_resolver(name: &str) -> DocumentIdResolver {
    let manifest_path = std::env::temp_dir().join("persisted-docs-bench.json");
    std::fs::write(&manifest_path, "{}").expect("bench manifest should be writable");

    let raw = format!(
        r#"{{
  "enabled": true,
  "storage": {{
    "type": "file",
    "path": "{}",
    "watch": false
  }},
  "selectors": [
    {{ "type": "url_query_param", "name": "{name}" }}
  ]
}}"#,
        manifest_path.display()
    );

    let config: PersistedDocumentsConfig =
        serde_json::from_str(&raw).expect("bench config should parse");
    DocumentIdResolver::from_config(&config, GRAPHQL_ENDPOINT)
        .expect("resolver config should compile")
}

fn persisted_documents_matcher_benchmark(c: &mut Criterion) {
    let cases = [
        PathCase {
            name: "simple_id",
            template: "/p/:id",
            hit_path: "/graphql/p/abc-123",
            miss_path: "/graphql/p",
        },
        PathCase {
            name: "single_wildcard",
            template: "/v1/*/:id/details",
            hit_path: "/graphql/v1/mobile/abc-123/details",
            miss_path: "/graphql/v1/mobile/abc-123",
        },
    ];

    let graphql_params = GraphQLParams::default();

    for case in cases {
        let resolver = build_resolver(case.template);

        let hit_context = HttpRequestContext::from_parts(case.hit_path, None);
        let miss_context = HttpRequestContext::from_parts(case.miss_path, None);

        let mut group = c.benchmark_group(format!("persisted_docs_path_match/{}", case.name));

        group.bench_with_input(
            BenchmarkId::new("current", "hit"),
            &hit_context,
            |b, ctx| {
                b.iter(|| {
                    let input = DocumentIdResolverInput {
                        graphql_params: &graphql_params,
                        document_id: None,
                        nonstandard_json_fields: None,
                        request_context: ctx,
                    };
                    black_box(resolver.resolve_document_id(input))
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("current", "miss"),
            &miss_context,
            |b, ctx| {
                b.iter(|| {
                    let input = DocumentIdResolverInput {
                        graphql_params: &graphql_params,
                        document_id: None,
                        nonstandard_json_fields: None,
                        request_context: ctx,
                    };
                    black_box(resolver.resolve_document_id(input))
                })
            },
        );

        group.finish();
    }
}

fn persisted_documents_query_param_benchmark(c: &mut Criterion) {
    let resolver = build_query_param_resolver("documentId");
    let graphql_params = GraphQLParams::default();

    let hit_query = "documentId=sha256:abc";
    let miss_query = "foo=bar";
    let long_miss_query =
        "a=1&b=2&c=3&d=4&e=5&f=6&g=7&h=8&i=9&j=10&k=11&l=12&m=13&n=14&o=15&p=16&q=17&r=18&s=19&t=20";
    let encoded_hit_query = "documentId=sha256%3Aabc";

    let mut group = c.benchmark_group("persisted_docs_query_param/current");

    group.bench_with_input(
        BenchmarkId::new("lookup", "hit_plain"),
        &hit_query,
        |b, query| {
            b.iter(|| {
                let ctx = HttpRequestContext::from_parts("/graphql", Some(query));
                let input = DocumentIdResolverInput {
                    graphql_params: &graphql_params,
                    document_id: None,
                    nonstandard_json_fields: None,
                    request_context: &ctx,
                };
                black_box(resolver.resolve_document_id(input))
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("lookup", "miss_plain"),
        &miss_query,
        |b, query| {
            b.iter(|| {
                let ctx = HttpRequestContext::from_parts("/graphql", Some(query));
                let input = DocumentIdResolverInput {
                    graphql_params: &graphql_params,
                    document_id: None,
                    nonstandard_json_fields: None,
                    request_context: &ctx,
                };
                black_box(resolver.resolve_document_id(input))
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("lookup", "miss_long"),
        &long_miss_query,
        |b, query| {
            b.iter(|| {
                let ctx = HttpRequestContext::from_parts("/graphql", Some(query));
                let input = DocumentIdResolverInput {
                    graphql_params: &graphql_params,
                    document_id: None,
                    nonstandard_json_fields: None,
                    request_context: &ctx,
                };
                black_box(resolver.resolve_document_id(input))
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("lookup", "hit_encoded"),
        &encoded_hit_query,
        |b, query| {
            b.iter(|| {
                let ctx = HttpRequestContext::from_parts("/graphql", Some(query));
                let input = DocumentIdResolverInput {
                    graphql_params: &graphql_params,
                    document_id: None,
                    nonstandard_json_fields: None,
                    request_context: &ctx,
                };
                black_box(resolver.resolve_document_id(input))
            })
        },
    );

    group.finish();
}

criterion_group!(
    benches,
    persisted_documents_matcher_benchmark,
    persisted_documents_query_param_benchmark,
);
criterion_main!(benches);
