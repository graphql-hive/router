//! The flat parser and printer against the tree ones: the same documents accepted, the same
//! minified text, the same positions.

use std::path::Path;

use super::{parse, parse_with_limits, Definition, Document, ListId, Pos};
use crate::parser::query::{
    parse_query, parse_query_with_token_limit, Definition as TreeDefinition, Directive,
    OperationDefinition, Selection, SelectionSet, VariableDefinition,
};
use crate::parser::{minify_query_document, Pos as TreePos};

type TreeDocument = crate::parser::query::Document<'static, String>;

fn pos(p: TreePos) -> Pos {
    Pos {
        line: p.line as u32,
        column: p.column as u32,
    }
}

/// Positions in document order: definitions, variables, directives, selections.
fn tree_positions(doc: &TreeDocument) -> Vec<(&'static str, Pos)> {
    fn directives(out: &mut Vec<(&'static str, Pos)>, ds: &[Directive<'static, String>]) {
        out.extend(ds.iter().map(|d| ("directive", pos(d.position))));
    }
    fn variables(out: &mut Vec<(&'static str, Pos)>, vs: &[VariableDefinition<'static, String>]) {
        out.extend(vs.iter().map(|v| ("variable", pos(v.position))));
    }
    fn set(out: &mut Vec<(&'static str, Pos)>, s: &SelectionSet<'static, String>) {
        for item in &s.items {
            match item {
                Selection::Field(f) => {
                    out.push(("field", pos(f.position)));
                    directives(out, &f.directives);
                    set(out, &f.selection_set);
                }
                Selection::FragmentSpread(f) => {
                    out.push(("spread", pos(f.position)));
                    directives(out, &f.directives);
                }
                Selection::InlineFragment(f) => {
                    out.push(("inline", pos(f.position)));
                    directives(out, &f.directives);
                    set(out, &f.selection_set);
                }
            }
        }
    }
    let mut out = Vec::new();
    for definition in &doc.definitions {
        match definition {
            TreeDefinition::Operation(op) => match op {
                OperationDefinition::SelectionSet(s) => {
                    out.push(("operation", pos(s.span.0)));
                    set(&mut out, s);
                }
                OperationDefinition::Query(q) => {
                    out.push(("operation", pos(q.position)));
                    variables(&mut out, &q.variable_definitions);
                    directives(&mut out, &q.directives);
                    set(&mut out, &q.selection_set);
                }
                OperationDefinition::Mutation(q) => {
                    out.push(("operation", pos(q.position)));
                    variables(&mut out, &q.variable_definitions);
                    directives(&mut out, &q.directives);
                    set(&mut out, &q.selection_set);
                }
                OperationDefinition::Subscription(q) => {
                    out.push(("operation", pos(q.position)));
                    variables(&mut out, &q.variable_definitions);
                    directives(&mut out, &q.directives);
                    set(&mut out, &q.selection_set);
                }
            },
            TreeDefinition::Fragment(f) => {
                out.push(("fragment", pos(f.position)));
                directives(&mut out, &f.directives);
                set(&mut out, &f.selection_set);
            }
        }
    }
    out
}

fn flat_positions(doc: &Document) -> Vec<(&'static str, Pos)> {
    fn directives(out: &mut Vec<(&'static str, Pos)>, doc: &Document, span: super::Span) {
        out.extend(doc.directives_of(span).iter().map(|d| ("directive", d.pos)));
    }
    fn set(out: &mut Vec<(&'static str, Pos)>, doc: &Document, list: ListId) {
        for &id in doc.items(list) {
            let s = doc.selection(id);
            let kind = match s.kind {
                super::Kind::Field => "field",
                super::Kind::FragmentSpread => "spread",
                super::Kind::InlineFragment => "inline",
            };
            out.push((kind, s.pos));
            directives(out, doc, s.directives);
            set(out, doc, s.children);
        }
    }
    let mut out = Vec::new();
    for definition in &doc.definitions {
        match definition {
            Definition::Operation(op) => {
                out.push(("operation", op.pos));
                out.extend(
                    doc.variables[op.variables.range()]
                        .iter()
                        .map(|v| ("variable", v.pos)),
                );
                directives(&mut out, doc, op.directives);
                set(&mut out, doc, op.selection_set);
            }
            Definition::Fragment(f) => {
                out.push(("fragment", f.pos));
                directives(&mut out, doc, f.directives);
                set(&mut out, doc, f.selection_set);
            }
        }
    }
    out
}

/// Both parsers agree on `src`: both fail, or both succeed with the same minified text and the
/// same positions.
fn check(src: &str, label: &str) -> bool {
    let tree = parse_query::<String>(src).map(|doc| doc.into_static());
    let flat = parse(src);
    match (tree, flat) {
        (Err(_), Err(_)) => false,
        (Ok(tree), Ok(flat)) => {
            assert_eq!(
                super::minify(&flat),
                minify_query_document(&tree),
                "minified text differs for {label}"
            );
            assert_eq!(
                flat_positions(&flat),
                tree_positions(&tree),
                "positions differ for {label}"
            );
            true
        }
        (Ok(_), Err(err)) => panic!("tree parses {label}, flat fails: {err}"),
        (Err(err), Ok(_)) => panic!("flat parses {label}, tree fails: {err}"),
    }
}

const CASES: &[&str] = &[
    "{ a }",
    "{ a b c }",
    "query { a }",
    "query Q { a }",
    "query query { a }",
    "query Q($a: Int, $b: [String!]! = [\"x\"], $c: In = {a: 1, b: [1, 2]}) { f(a: $a) }",
    "query Q(\"described\" $a: Int, \"\"\"block\"\"\" $b: String) { f }",
    "mutation M @d(a: 1) @e { f }",
    "subscription { f }",
    "{ alias: f(a: 1, b: \"s\", c: 1.5, d: -0, e: true, f: null, g: ENUM, h: [1 2 3], i: {x: $v}) }",
    "{ f @include(if: $x) @skip(if: false) }",
    "{ ... on T { a } ... @d { b } ... { c } ...F ...F @d }",
    "fragment F on T @d { a }",
    "fragment F on T { a } query { ...F }",
    "{ a { b { c { d } } } }",
    "{\n\ta\n\t\tb # comment\n  c }",
    "{ a }\r\n# comment\r\n{ b }",
    "\u{feff}{ a }",
    "{ f(a: \"\\u0041\\n\\t\\\"\\\\\\/\\b\\f\\r\") }",
    "{ f(a: \"\"\"\n    block\n      indented\n    \\\"\"\" done\n  \"\"\") }",
    "{ f(a: \"emoji 😀 and ünïcödé\") g }",
    "{ f(a: 1e3, b: 1.5e-3, c: -1.0E+2, d: 0.0) }",
    "{ f(a: 9223372036854775807) }",
    // rejected by both
    "",
    "{ }",
    "{ a(): b }",
    "{ f(a: 9223372036854775808) }",
    "{ f(a: 01) }",
    "{ f(a: 1.) }",
    "{ f(a: 1e) }",
    "{ f(a: 1a) }",
    "{ f(a: \"unterminated) }",
    "{ f(a: \"line\nbreak\") }",
    "{ a } garbage",
    "query ($a: Int @d) { a }",
    "query ($a: Int = $b) { a }",
    "{ ...on }",
    "{ ... on { a } }",
    "{ a . b }",
    "{ a ; }",
    "fragment F { a }",
    "type Query { a: String }",
    "schema { query: Query }",
];

#[test]
fn cases_agree_with_the_tree_parser() {
    for case in CASES {
        check(case, &format!("{case:?}"));
    }
}

#[test]
fn deep_nesting_hits_the_same_limit() {
    for depth in [48, 49, 50, 51] {
        let src = format!("{}a{}", "{ a ".repeat(depth), " }".repeat(depth));
        check(&src, &format!("depth {depth}"));
    }
}

#[test]
fn token_limits_agree() {
    let src = "query Q($a: Int) { a(x: $a) { b c } d }";
    for limit in 0..30 {
        let tree = parse_query_with_token_limit::<String>(src, limit).is_ok();
        let flat = parse_with_limits(src, Some(limit)).is_ok();
        assert_eq!(flat, tree, "token limit {limit}");
    }
}

/// Every `.graphql` file in the repository: operations must come out the same, schemas must be
/// rejected by both.
#[test]
fn repository_documents_agree_with_the_tree_parser() {
    fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !matches!(name.as_ref(), "target" | "node_modules" | ".git") {
                    walk(&path, files);
                }
            } else if name.ends_with(".graphql") {
                files.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    walk(&root, &mut files);
    assert!(!files.is_empty());
    let mut parsed = 0;
    for file in &files {
        let Ok(src) = std::fs::read_to_string(file) else {
            continue;
        };
        if check(&src, &file.display().to_string()) {
            parsed += 1;
        }
    }
    assert!(parsed > 50, "only {parsed} operations parsed");
}

/// Every prefix of every case and of the benchmark operation: the two parsers agree on what's
/// a document, on cut-off input too.
#[test]
fn every_prefix_agrees() {
    let bench = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/operation.graphql"),
    )
    .unwrap();
    let mut sources: Vec<&str> = CASES.to_vec();
    sources.push(&bench);
    for src in sources {
        for (end, _) in src.char_indices().skip(1) {
            check(&src[..end], &format!("prefix {end} of {src:?}"));
        }
    }
}
