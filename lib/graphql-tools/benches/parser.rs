use criterion::{criterion_group, criterion_main, Criterion};
use graphql_tools::parser::minify_query_document;
use graphql_tools::parser::{minify_query, parse_query, query::Document};
use std::fs::File;
use std::hint::black_box;
use std::io::Read;

fn load_file(name: &str) -> String {
    let mut buf = String::with_capacity(1024);
    let path = format!("./src/parser/tests/queries/{}.graphql", name);
    let mut f = File::open(&path).unwrap_or_else(|_| panic!("failed to open file {}", path));
    f.read_to_string(&mut buf).unwrap();
    buf
}

fn bench_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser");

    let cases = [
        "minimal",
        "inline_fragment",
        "directive_args",
        "query_vars",
        "kitchen-sink",
    ];

    for name in cases {
        let content = load_file(name);
        group.bench_function(format!("{}/String", name), |b| {
            b.iter(|| {
                parse_query::<String>(black_box(content.as_str())).expect("failed to parse query")
            });
        });
        group.bench_function(format!("{}/&str", name), |b| {
            b.iter(|| {
                parse_query::<&str>(black_box(content.as_str())).expect("failed to parse query")
            });
        });
        group.bench_function(format!("{}/flat", name), |b| {
            b.iter(|| {
                graphql_tools::flat::parse(black_box(content.as_str()))
                    .expect("failed to parse query")
            });
        });
    }
    let bench = std::fs::read_to_string("../../bench/operation.graphql").unwrap();
    group.bench_function("bench_operation/String", |b| {
        b.iter(|| parse_query::<String>(black_box(bench.as_str())).expect("failed to parse query"))
    });
    group.bench_function("bench_operation/flat", |b| {
        b.iter(|| graphql_tools::flat::parse(black_box(bench.as_str())).expect("failed to parse"))
    });

    group.finish();
}

fn bench_minifiers(c: &mut Criterion) {
    let mut group = c.benchmark_group("minifiers");
    let query = load_file("kitchen-sink");

    let parsed: Document<'_, String> =
        black_box(parse_query(query.as_str()).expect("failed to parse query"));
    let source = parsed.to_string();

    group.bench_function("minify_query", |b| {
        b.iter(|| minify_query(black_box(source.as_str())).expect("failed to minify query"))
    });

    group.bench_function("minify_document", |b| {
        b.iter(|| minify_query_document(black_box(&parsed)))
    });

    let flat = graphql_tools::flat::parse(&source).expect("failed to parse query");
    group.bench_function("minify_flat", |b| {
        b.iter(|| graphql_tools::flat::minify(black_box(&flat)))
    });

    group.finish();
}

criterion_group!(benches, bench_parser, bench_minifiers);
criterion_main!(benches);
