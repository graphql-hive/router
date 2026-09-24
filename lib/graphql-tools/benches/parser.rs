use criterion::{criterion_group, criterion_main, Criterion};
use graphql_tools::parser::minify_query_document;
use graphql_tools::parser::{minify_query, parse_query, query::Document};
use std::fs::File;
use std::hint::black_box;
use std::io::Read;

fn load_file(name: &str) -> String {
    let mut buf = String::with_capacity(1024);
    let path = format!(
        "{}/src/parser/tests/queries/{}.graphql",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
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
    }

    let string_heavy = format!(
        "query Strings {{\n{}\n}}",
        (0..64)
            .map(|index| format!("field{index}(value: \"{}\")", "a".repeat(128)))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let whitespace_and_comments = format!(
        "query Whitespace {{{}}}",
        (0..128)
            .map(|index| format!(" # comment {index}\n field{index}\n"))
            .collect::<Vec<_>>()
            .join("")
    );

    for (name, content) in [
        ("string-heavy", string_heavy),
        ("whitespace-and-comments", whitespace_and_comments),
    ] {
        group.bench_function(format!("{name}/String"), |b| {
            b.iter(|| {
                parse_query::<String>(black_box(content.as_str())).expect("failed to parse query")
            });
        });
        group.bench_function(format!("{name}/&str"), |b| {
            b.iter(|| {
                parse_query::<&str>(black_box(content.as_str())).expect("failed to parse query")
            });
        });
    }

    let escaped_strings = format!(
        "query Escapes {{{}}}",
        (0..64)
            .map(|index| {
                format!("field{index}(value: \"quote\\\" slash\\\\ newline\\n unicode\\u263A\")")
            })
            .collect::<Vec<_>>()
            .join(" ")
    );
    let unicode_strings = format!(
        "query Unicode {{{}}}",
        (0..64)
            .map(|index| format!("field{index}(value: \"café漢字\")"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let block_strings = format!(
        "query Blocks {{{}}}",
        (0..32)
            .map(|index| format!("field{index}(value: \"\"\"\\n  block {index} \\\"\"\"\\n\"\"\")"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let numbers = format!(
        "query Numbers {{{}}}",
        (0..64)
            .map(|index| format!("field{index}(integer: {index}, float: {index}.5e+2)"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let nested = "query Nested { root { a { b { c { d { e } } } } } }".to_string();

    for (name, content) in [
        ("escaped-strings", escaped_strings),
        ("unicode-strings", unicode_strings),
        ("block-strings", block_strings),
        ("numbers", numbers),
        ("nested", nested),
    ] {
        group.bench_function(format!("{name}/String"), |b| {
            b.iter(|| parse_query::<String>(black_box(content.as_str())).expect("valid query"));
        });
        group.bench_function(format!("{name}/&str"), |b| {
            b.iter(|| parse_query::<&str>(black_box(content.as_str())).expect("valid query"));
        });
    }

    let token_limited = load_file("kitchen-sink");
    group.bench_function("token-limit/String", |b| {
        b.iter(|| {
            graphql_tools::parser::parse_query_with_token_limit::<String>(
                black_box(token_limited.as_str()),
                10_000,
            )
            .expect("valid query")
        });
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

    group.finish();
}

criterion_group!(benches, bench_parser, bench_minifiers);
criterion_main!(benches);
