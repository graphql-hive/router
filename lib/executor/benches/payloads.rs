#![allow(dead_code)]

//! Synthetic subgraph payloads for the deserialize/merge/requires benches.
//!
//! Each generator returns a full GraphQL response envelope so it can be fed straight
//! into `SubgraphResponse::deserialize_from_bytes`.

/// Objects made of short strings — the common "list of records" response.
pub fn scalar_heavy(rows: usize) -> String {
    let mut out = String::from(r#"{"data":{"users":["#);
    for i in 0..rows {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            r#"{{"__typename":"User","id":"user-{i}","name":"Name {i}","username":"user{i}","email":"user{i}@example.com","bio":"A reasonably long biography string for user {i} that exercises the escape scanner."}}"#
        ));
    }
    out.push_str("]}}");
    out
}

/// Numbers only — isolates `strtod` on parse and `write_f64`/`write_u64` on projection.
pub fn number_heavy(rows: usize) -> String {
    let mut out = String::from(r#"{"data":{"metrics":["#);
    for i in 0..rows {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            r#"{{"__typename":"Metric","id":"m{i}","count":{i},"ratio":{}.{},"total":{}}}"#,
            i,
            i % 97,
            i * 31
        ));
    }
    out.push_str("]}}");
    out
}

/// Long lists of leaves hanging off a few objects — the case raw passthrough targets.
pub fn scalar_lists(rows: usize, items_per_row: usize) -> String {
    let mut out = String::from(r#"{"data":{"products":["#);
    for i in 0..rows {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(r#"{{"__typename":"Product","id":"p{i}","tags":["#));
        for t in 0..items_per_row {
            if t > 0 {
                out.push(',');
            }
            out.push_str(&format!(r#""tag-{i}-{t}""#));
        }
        out.push_str(r#"],"scores":["#);
        for t in 0..items_per_row {
            if t > 0 {
                out.push(',');
            }
            out.push_str(&format!("{}", t * 7));
        }
        out.push_str("]}");
    }
    out.push_str("]}}");
    out
}

/// The realistic middle: mostly plain scalars with one leaf list. Only the list is a
/// passthrough, but the whole object still pays the shaped-visitor cost, so this is what
/// decides whether narrowing eligibility to aggregates actually pays.
pub fn mixed(rows: usize, tags_per_row: usize) -> String {
    let mut out = String::from(r#"{"data":{"articles":["#);
    for i in 0..rows {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            r#"{{"__typename":"Article","id":"a{i}","title":"Title {i}","author":"Author {i}","views":{i},"tags":["#
        ));
        for t in 0..tags_per_row {
            if t > 0 {
                out.push(',');
            }
            out.push_str(&format!(r#""tag-{t}""#));
        }
        out.push_str("]}");
    }
    out.push_str("]}}");
    out
}

/// Deeply nested objects — isolates per-object overhead (sort, alloc) over payload size.
pub fn deep_objects(depth: usize, breadth: usize) -> String {
    fn node(depth: usize, breadth: usize, counter: &mut usize, out: &mut String) {
        *counter += 1;
        out.push_str(r#"{"__typename":"Node","id":"n"#);
        out.push_str(&counter.to_string());
        out.push('"');
        if depth > 0 {
            out.push_str(r#","children":["#);
            for i in 0..breadth {
                if i > 0 {
                    out.push(',');
                }
                node(depth - 1, breadth, counter, out);
            }
            out.push(']');
        }
        out.push('}');
    }
    let mut out = String::from(r#"{"data":{"root":"#);
    node(depth, breadth, &mut 0, &mut out);
    out.push_str("}}");
    out
}

/// Builds a `ResponseShape` the way the planner does: every response key present, in
/// selection order, with the raw ones marked. Shapes built by marking only a few paths are
/// not representative — they force a scan miss on every unlisted key.
pub fn shape(spec: &[(&str, Shape)]) -> hive_router_query_planner::planner::response_shape::ResponseShape {
    use hive_router_query_planner::planner::response_shape::{ResponseShape, ResponseShapeField};
    let fields: Vec<ResponseShapeField> = spec
        .iter()
        .map(|(key, shape)| ResponseShapeField {
            key: (*key).to_string(),
            shape: match shape {
                Shape::Raw => ResponseShape {
                    fields: Vec::new(),
                    raw: true,
                    inert: false,
                },
                Shape::Structured => ResponseShape {
                    fields: Vec::new(),
                    raw: false,
                    inert: true,
                },
                Shape::Nested(inner) => self::shape(inner),
            },
        })
        .collect();
    let inert = fields.iter().all(|f| f.shape.inert);
    ResponseShape {
        fields,
        raw: false,
        inert,
    }
}

pub enum Shape<'a> {
    Raw,
    Structured,
    Nested(&'a [(&'a str, Shape<'a>)]),
}
