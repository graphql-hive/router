use crate::query_planner::utils::parsing::parse_operation;

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
