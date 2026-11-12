//! The flat rules against the tree ones. The corpus is every string literal in the
//! `graphql_tools` rule tests and the router's limit rule tests: the ones that parse as an
//! operation are run against the ones that parse as a schema, by both validators, and the errors
//! must come out the same, in the same order, at the same places.
//!
//! `FLAT_RULES=KnownArgumentNames,LeafFieldSelections` checks only those rules.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use graphql_tools::parser::{parse_query, parse_schema};
use graphql_tools::static_graphql::schema;
use graphql_tools::validation::rules::{
    default_rules_validation_plan_with_options, ValidationOptions, ValidationRule,
};
use graphql_tools::validation::utils::ValidationError;
use graphql_tools::validation::validate::{validate, ValidationPlan};

use super::rules;
use super::validate_flat;
use super::visit::FlatRule;
use crate::config::limits::{MaxAliasesRuleConfig, MaxDepthRuleConfig, MaxDirectivesRuleConfig};
use crate::pipeline::validation::{
    max_aliases_rule::MaxAliasesRule, max_depth_rule::MaxDepthRule,
    max_directives_rule::MaxDirectivesRule,
};

/// Every string literal in a Rust source, unescaped.
fn string_literals(src: &str) -> Vec<String> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'\'' => {
                // a char literal, or a lifetime
                if b.get(i + 1) == Some(&b'\\') {
                    i += 2;
                    while i < b.len() && b[i] != b'\'' {
                        i += 1;
                    }
                    i += 1;
                } else if b.get(i + 2) == Some(&b'\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            b'r' if (i == 0 || !ident(b[i - 1]))
                && matches!(b.get(i + 1), Some(b'"') | Some(b'#')) =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while b.get(j) == Some(&b'#') {
                    hashes += 1;
                    j += 1;
                }
                if b.get(j) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                let start = j + 1;
                let close = format!("\"{}", "#".repeat(hashes));
                let end = start + src[start..].find(&close).unwrap();
                out.push(src[start..end].to_string());
                i = end + close.len();
            }
            b'"' => {
                let mut s = String::new();
                let mut chars = src[i + 1..].char_indices();
                let mut end = None;
                while let Some((at, c)) = chars.next() {
                    match c {
                        '"' => {
                            end = Some(i + 1 + at + 1);
                            break;
                        }
                        '\\' => match chars.next().map(|(_, c)| c) {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('r') => s.push('\r'),
                            Some('0') => s.push('\0'),
                            Some('u') => {
                                let mut hex = String::new();
                                for (_, c) in chars.by_ref() {
                                    match c {
                                        '{' => {}
                                        '}' => break,
                                        c => hex.push(c),
                                    }
                                }
                                s.push(
                                    char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap(),
                                );
                            }
                            Some('\n') => {
                                // a line continuation skips the next line's leading whitespace
                                while chars.clone().next().is_some_and(|(_, c)| c.is_whitespace()) {
                                    chars.next();
                                }
                            }
                            Some(c) => s.push(c),
                            None => break,
                        },
                        c => s.push(c),
                    }
                }
                out.push(s);
                i = end.unwrap_or(b.len());
            }
            _ => i += 1,
        }
    }
    out
}

struct Corpus {
    operations: Vec<String>,
    schemas: Vec<schema::Document>,
}

fn corpus() -> Corpus {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in [
        root.join("../../lib/graphql-tools/src/validation/rules"),
        root.join("../../lib/graphql-tools/src/validation"),
        root.join("src/pipeline/validation"),
    ] {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            if entry.path().extension().is_some_and(|e| e == "rs") {
                files.push(entry.path());
            }
        }
    }
    let literals: Vec<String> = files
        .iter()
        .flat_map(|f| string_literals(&std::fs::read_to_string(f).unwrap()))
        .collect();
    // the tests' `INTROSPECTION_SCHEMA`, which they add to every schema
    let introspection = literals
        .iter()
        .find(|s| s.contains("type __Schema") && s.contains("type Query"))
        .expect("the introspection schema")
        .clone();

    let mut operations = Vec::new();
    let mut schema_sources = Vec::new();
    {
        for s in literals {
            if parse_query::<String>(&s).is_ok() && graphql_tools::flat::parse(&s).is_ok() {
                operations.push(s);
            } else if parse_schema::<String>(&s).is_ok_and(|d| !d.definitions.is_empty()) {
                schema_sources.push(s.clone());
                if !s.contains("__Schema") {
                    schema_sources.push(s + &introspection);
                }
            }
        }
    }
    operations.sort();
    operations.dedup();
    schema_sources.sort();
    schema_sources.dedup();

    let schemas = schema_sources
        .into_iter()
        .filter_map(|s| {
            let doc = parse_schema::<String>(&s).ok()?.into_static();
            let query = doc.query_type_name()?;
            matches!(
                doc.type_by_name(query),
                Some(schema::TypeDefinition::Object(_))
            )
            .then_some(doc)
        })
        .collect();
    Corpus {
        operations,
        schemas,
    }
}

type Seen = Vec<(&'static str, String, Vec<(usize, usize)>)>;

/// Rules whose tree order comes from a `HashMap`, so it changes from run to run; the flat ones
/// report in document order. Their errors are compared as a set.
const ORDER_FREE: &[&str] = &[
    "NoUnusedVariables",
    "NoUndefinedVariables",
    "NoUnusedFragments",
    "UniqueOperationNames",
    "UniqueFragmentNames",
];

fn seen(errors: &[ValidationError]) -> Seen {
    let mut out: Seen = errors
        .iter()
        .map(|e| {
            (
                e.error_code,
                e.message.clone(),
                e.locations.iter().map(|p| (p.line, p.column)).collect(),
            )
        })
        .collect();
    // the order-free ones go last, sorted; the rest keep their order
    out.sort_by(|a, b| {
        let (fa, fb) = (ORDER_FREE.contains(&a.0), ORDER_FREE.contains(&b.0));
        match (fa, fb) {
            (true, true) => a.cmp(b),
            _ => fa.cmp(&fb),
        }
    });
    out
}

/// Runs the tree plan and the flat rules on the whole corpus; returns how many errors each
/// error code reported.
fn compare(
    corpus: &Corpus,
    tree: &ValidationPlan,
    flat: &[&dyn FlatRule],
) -> std::collections::BTreeMap<&'static str, usize> {
    let mut counts = std::collections::BTreeMap::new();
    let mut failures = Vec::new();
    for schema in &corpus.schemas {
        for op in &corpus.operations {
            let tree_doc = parse_query::<String>(op).unwrap().into_static();
            let flat_doc = graphql_tools::flat::parse(op).unwrap();
            let expected = catch_unwind(AssertUnwindSafe(|| {
                seen(&validate(schema, &tree_doc, tree))
            }));
            let actual = catch_unwind(AssertUnwindSafe(|| {
                seen(&validate_flat(schema, &flat_doc, flat))
            }));
            match (expected, actual) {
                (Ok(expected), Ok(actual)) => {
                    for (code, _, _) in &expected {
                        *counts.entry(*code).or_insert(0) += 1;
                    }
                    if expected != actual && failures.len() < 10 {
                        failures.push(format!(
                            "operation:\n{op}\nschema starts:\n{}\ntree: {expected:#?}\nflat: {actual:#?}",
                            schema.to_string().chars().take(300).collect::<String>()
                        ));
                    }
                }
                (Err(_), Err(_)) => {}
                (Ok(_), Err(_)) if failures.len() < 10 => {
                    failures.push(format!("flat panics, tree doesn't:\n{op}"))
                }
                (Err(_), Ok(_)) if failures.len() < 10 => {
                    failures.push(format!("tree panics, flat doesn't:\n{op}"))
                }
                _ => {}
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n---\n\n"));
    counts
}

fn selected(code: &str) -> bool {
    match std::env::var("FLAT_RULES") {
        Ok(list) => list.split(',').any(|c| c.trim() == code),
        Err(_) => true,
    }
}

/// Both sides as plans of the selected rules, in the same order.
fn plans(
    tree: Vec<Box<dyn ValidationRule>>,
    flat: Vec<Box<dyn FlatRule>>,
) -> (ValidationPlan, Vec<Box<dyn FlatRule>>) {
    let tree: Vec<_> = tree
        .into_iter()
        .filter(|r| selected(r.error_code()))
        .collect();
    let flat = flat
        .into_iter()
        .filter(|r| selected(r.error_code()))
        .collect();
    (ValidationPlan::from(tree), flat)
}

fn depth(n: usize, ignore_introspection: bool, flatten_fragments: bool) -> MaxDepthRuleConfig {
    MaxDepthRuleConfig {
        n,
        ignore_introspection,
        flatten_fragments,
    }
}

#[test]
fn flat_rules_report_what_the_tree_rules_report() {
    let corpus = corpus();
    assert!(corpus.operations.len() > 300, "{}", corpus.operations.len());
    assert!(corpus.schemas.len() > 5, "{}", corpus.schemas.len());

    let options = ValidationOptions::default();
    let tree_plan = default_rules_validation_plan_with_options(options);
    let mut tree_plan = ValidationPlan {
        rules: tree_plan
            .rules
            .into_iter()
            .filter(|r| selected(r.error_code()))
            .collect(),
        hash: 0,
    };
    let mut flat_rules: Vec<Box<dyn FlatRule>> = rules::defaults(options)
        .into_iter()
        .filter(|r| selected(r.error_code()))
        .collect();
    let (limits_tree, limits_flat) = plans(
        vec![
            Box::new(MaxDepthRule {
                config: depth(3, true, false),
            }),
            Box::new(MaxDirectivesRule {
                config: MaxDirectivesRuleConfig { n: 1 },
            }),
            Box::new(MaxAliasesRule {
                config: MaxAliasesRuleConfig { n: 1 },
            }),
        ],
        vec![
            Box::new(rules::max_depth::MaxDepth {
                config: depth(3, true, false),
            }),
            Box::new(rules::max_directives::MaxDirectives {
                config: MaxDirectivesRuleConfig { n: 1 },
            }),
            Box::new(rules::max_aliases::MaxAliases {
                config: MaxAliasesRuleConfig { n: 1 },
            }),
        ],
    );
    tree_plan.rules.extend(limits_tree.rules);
    flat_rules.extend(limits_flat);

    let flat: Vec<&dyn FlatRule> = flat_rules.iter().map(|r| r.as_ref()).collect();
    let counts = compare(&corpus, &tree_plan, &flat);
    for rule in &flat {
        let code = rule.error_code();
        assert!(
            counts.get(code).is_some_and(|&n| n > 0),
            "the corpus never trips {code}: {counts:?}"
        );
    }

    // the other depth settings
    for config in [
        depth(2, false, true),
        depth(1, false, false),
        depth(4, true, true),
    ] {
        let (tree, flat) = plans(
            vec![Box::new(MaxDepthRule {
                config: config.clone(),
            })],
            vec![Box::new(rules::max_depth::MaxDepth { config })],
        );
        let flat: Vec<&dyn FlatRule> = flat.iter().map(|r| r.as_ref()).collect();
        compare(&corpus, &tree, &flat);
    }
}
