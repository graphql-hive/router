use criterion::{criterion_group, criterion_main, Criterion};
use graphql_tools::ast::{collect_field_references, collect_fields, OperationVisitorContext};
use graphql_tools::parser::minify_query_document;
use graphql_tools::parser::{minify_query, parse_query, query::Document};
use graphql_tools::parser::{parse_schema, query, schema};
use graphql_tools::validation::rules::{OverlappingFieldsCanBeMerged, ValidationRule};
use graphql_tools::validation::validate::{validate, ValidationPlan};
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

fn bench_formatters(c: &mut Criterion) {
    let mut group = c.benchmark_group("formatters");

    let numbers = format!(
        "query Numbers {{{}}}",
        (0..64)
            .map(|index| format!("field{index}(integer: {index}, float: {index}.5e+2)"))
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

    for (name, source) in [
        ("kitchen-sink", load_file("kitchen-sink")),
        ("numbers", numbers),
        ("block-strings", block_strings),
    ] {
        let parsed: Document<'_, String> =
            parse_query(source.as_str()).expect("failed to parse query");

        group.bench_function(name, |b| {
            b.iter(|| black_box(&parsed).to_string());
        });
    }

    group.finish();
}

fn bench_field_collection(c: &mut Criterion) {
    let mut group = c.benchmark_group("field-collection");
    let query_source = r#"
        query Benchmark {
            alpha(a: 123, filter: {name: "a", tags: ["a", "b"]}) {
                id
                details { value }
            }
            alpha(a: 456, filter: {name: "b", tags: ["c", "d"]}) {
                id
                details { value }
            }
            ...Reusable
        }
        fragment Reusable on Query {
            beta { id details { value } }
        }
    "#;
    let schema_source = "type Query { alpha: String beta: String }";
    let operation = parse_query::<String>(query_source)
        .expect("failed to parse collection query")
        .into_static();
    let schema = parse_schema::<String>(schema_source)
        .expect("failed to parse collection schema")
        .into_static();
    let context = OperationVisitorContext::new(&operation, &schema);
    let parent_type = schema::TypeDefinition::Object(schema::ObjectType::new("Query".to_owned()));
    let selection_set = match &operation.definitions[0] {
        query::Definition::Operation(query::OperationDefinition::Query(operation)) => {
            &operation.selection_set
        }
        _ => panic!("expected query operation"),
    };

    group.bench_function("owned", |b| {
        b.iter(|| {
            collect_fields(
                black_box(selection_set),
                black_box(&parent_type),
                &context.known_fragments,
                black_box(&context),
            )
        });
    });
    group.bench_function("borrowed", |b| {
        b.iter(|| {
            collect_field_references(
                black_box(selection_set),
                black_box(&parent_type),
                &context.known_fragments,
                black_box(&context),
            )
        });
    });

    group.finish();
}

fn bench_overlapping_fields(c: &mut Criterion) {
    let mut group = c.benchmark_group("validation");
    let fragment_count = 16;
    let spreads = (0..fragment_count)
        .map(|index| format!("...F{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let fragments = (0..fragment_count)
        .map(|index| format!("fragment F{index} on Query {{ value }}"))
        .collect::<Vec<_>>()
        .join(" ");
    let query_source = format!("{{ {spreads} }} {fragments}");
    let operation = parse_query::<String>(&query_source)
        .expect("failed to parse validation query")
        .into_static();
    let schema = parse_schema::<String>("type Query { value: String }")
        .expect("failed to parse validation schema")
        .into_static();
    let plan = ValidationPlan::from(vec![
        Box::new(OverlappingFieldsCanBeMerged::new()) as Box<dyn ValidationRule>
    ]);

    group.bench_function("overlapping-fragments", |b| {
        b.iter(|| black_box(validate(&schema, &operation, &plan)));
    });

    group.finish();
}

fn bench_possible_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("schema");
    let schema = parse_schema::<String>(
        "interface Node { id: ID } \
         type User implements Node { id: ID } \
         type Product implements Node { id: ID } \
         type Query { node: Node }",
    )
    .expect("failed to parse possible-types schema")
    .into_static();
    let interface = schema
        .definitions
        .iter()
        .find_map(|definition| match definition {
            schema::Definition::TypeDefinition(type_def @ schema::TypeDefinition::Interface(_)) => {
                Some(type_def)
            }
            _ => None,
        })
        .expect("expected Node interface");

    group.bench_function("possible-types", |b| {
        b.iter(|| black_box(interface).possible_types(black_box(&schema)));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parser,
    bench_minifiers,
    bench_formatters,
    bench_field_collection,
    bench_overlapping_fields,
    bench_possible_types
);
criterion_main!(benches);
