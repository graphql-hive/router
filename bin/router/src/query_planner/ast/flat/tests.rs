use crate::query_planner::ast::normalization::{normalize_operation, normalize_operation_tree};
use crate::query_planner::state::supergraph_state::SupergraphState;
use crate::query_planner::utils::parsing::{parse_operation, parse_schema};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/");

/// Every fixture operation normalizes the same on the flat AST as with the passes on the tree.
#[test]
fn flat_normalization_matches_tree_on_fixtures() {
    let fixtures: &[(&str, &[&str])] = &[
        (
            "../../bench/supergraph.graphql",
            &["../../bench/operation.graphql"],
        ),
        (
            "fixture/abstract-many-subgraphs/supergraph.graphql",
            &["fixture/abstract-many-subgraphs/operation.graphql"],
        ),
        (
            "fixture/bfg/supergraph.graphql",
            &[
                "fixture/bfg/operation-1.graphql",
                "fixture/bfg/operation-2.graphql",
            ],
        ),
        (
            "fixture/grafbase-many-plans/supergraph.graphql",
            &["fixture/grafbase-many-plans/operation.graphql"],
        ),
        (
            "fixture/heavy-query/supergraph.graphql",
            &["fixture/heavy-query/operation.graphql"],
        ),
        (
            "fixture/products-list/supergraph.graphql",
            &["fixture/products-list/operation.graphql"],
        ),
        ("fixture/supergraph.graphql", &["fixture/operation.graphql"]),
        (
            "fixture/wfg/supergraph.graphql",
            &["fixture/wfg/query-2.graphql"],
        ),
    ];
    for (supergraph, operations) in fixtures {
        let sdl = std::fs::read_to_string(format!("{ROOT}{supergraph}")).unwrap();
        let state = SupergraphState::new(&parse_schema(&sdl));
        for operation in *operations {
            let query = std::fs::read_to_string(format!("{ROOT}{operation}")).unwrap();
            let document = parse_operation(&query);
            let flat = normalize_operation(&state, &document, None).unwrap();
            let tree = normalize_operation_tree(&state, &document, None).unwrap();
            assert_eq!(flat.operation_name, tree.operation_name, "{operation}");
            assert_eq!(
                format!("{:?}", flat.operation),
                format!("{:?}", tree.operation),
                "{operation}"
            );
        }
    }
}

/// The flat parser reads the text into the same document the tree converts into.
#[test]
fn flat_parse_matches_tree_conversion() {
    let query = r#"
        query Q($a: [Int!]! = [1, 2], "described" $b: String) @op {
            alias: field(x: {b: 1, a: "s\n", a: 2.5}, y: ENUM, z: null, w: $a) @skip(if: false) {
                ... on T @include(if: $b) { id }
                ... { name }
                ...F
            }
        }
        fragment F on T { other(arg: """ block
          string """) }
    "#;
    let from_text = super::parse::parse(query).unwrap();
    let from_tree = super::tree::from_tree(&parse_operation(query));
    assert_eq!(
        format!("{:?}", flat_shape(&from_text)),
        format!("{:?}", flat_shape(&from_tree))
    );
}

/// The document's contents with the names written out, so two interners compare.
fn flat_shape(doc: &super::Document) -> Vec<String> {
    use super::{Definition, Value};
    let value = |id: u32| -> String {
        fn render(doc: &super::Document, id: u32) -> String {
            match doc.values[id as usize] {
                Value::Variable(s) => format!("${}", doc.str(s)),
                Value::String(s) => format!("{:?}", doc.str(s)),
                Value::Enum(s) => doc.str(s).to_string(),
                Value::List(span) => format!(
                    "[{}]",
                    doc.value_items[span.range()]
                        .iter()
                        .map(|item| render(doc, *item))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                Value::Object(span) => format!(
                    "{{{}}}",
                    doc.object_fields[span.range()]
                        .iter()
                        .map(|(name, item)| format!("{}:{}", doc.str(*name), render(doc, *item)))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                other => format!("{other:?}"),
            }
        }
        render(doc, id)
    };
    let name = |s: u32| {
        if s == super::NONE {
            "-".to_string()
        } else {
            doc.str(s).to_string()
        }
    };
    let mut out = Vec::new();
    for definition in &doc.definitions {
        let (header, list) = match definition {
            Definition::Operation(op) => {
                let variables: Vec<String> = doc.variables[op.variables.range()]
                    .iter()
                    .map(|v| format!("{}={:?}", name(v.name), v.default_value.map(value)))
                    .collect();
                (
                    format!("op {:?} {} {variables:?}", op.kind, name(op.name)),
                    op.selection_set,
                )
            }
            Definition::Fragment(f) => (
                format!("fragment {} on {}", name(f.name), name(f.type_condition)),
                f.selection_set,
            ),
        };
        out.push(header);
        let mut stack = vec![(0, list)];
        while let Some((depth, list)) = stack.pop() {
            for id in doc.children[list.range()].iter().rev() {
                let s = doc.selections[*id as usize];
                let args: Vec<String> = doc.args[s.args.range()]
                    .iter()
                    .map(|a| format!("{}:{}", name(a.name), value(a.value)))
                    .collect();
                let directives: Vec<String> = doc.directives[s.directives.range()]
                    .iter()
                    .map(|d| {
                        let args: Vec<String> = doc.args[d.args.range()]
                            .iter()
                            .map(|a| format!("{}:{}", name(a.name), value(a.value)))
                            .collect();
                        format!("@{}{args:?}", name(d.name))
                    })
                    .collect();
                out.push(format!(
                    "{depth} {:?} {} {} {} {args:?} {directives:?}",
                    s.kind,
                    name(s.name),
                    name(s.alias),
                    name(s.type_condition)
                ));
                stack.push((depth + 1, s.children));
            }
        }
    }
    out
}
