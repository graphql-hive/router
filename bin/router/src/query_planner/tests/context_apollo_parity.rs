//! Context planning cases from Apollo's own suites that Hive Router had no test for.
//!
//! Each test names the Apollo test it ports, uses the same subgraph schemas composed with
//! `@apollo/composition`, and asserts the same property Apollo asserts about the context
//! rewrite: which ancestor it reads, how far up it walks, and which type conditions survive.
//! A plan snapshot sits beside each assertion so a planner change shows up as a diff.
//!
//! Source: apollographql/router `apollo-federation/tests/query_plan/build_query_plan_tests/
//! incremental_planner.rs` at 561720c2, and apollographql/federation `query-planner-js`
//! context tests.

use super::testkit::build_query_plan_with_defaults;
use crate::query_planner::utils::parsing::parse_operation;
use serde_json::Value;
use std::error::Error;

/// Every `contextRewrites` path in the plan, in plan order, as JSON.
fn context_rewrite_paths(plan: &Value) -> Vec<Vec<Value>> {
    fn walk(node: &Value, out: &mut Vec<Vec<Value>>) {
        match node {
            Value::Object(map) => {
                if let Some(Value::Array(rewrites)) = map.get("contextRewrites") {
                    for rewrite in rewrites {
                        if let Some(Value::Array(path)) = rewrite.get("path") {
                            out.push(path.clone());
                        }
                    }
                }
                for value in map.values() {
                    walk(value, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(plan, &mut out);
    out
}

/// How many `Fetch` nodes the plan has.
fn fetch_count(plan: &Value) -> usize {
    match plan {
        Value::Object(map) => {
            let own = usize::from(map.get("kind") == Some(&Value::String("Fetch".into())));
            own + map.values().map(fetch_count).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(fetch_count).sum(),
        _ => 0,
    }
}

fn plan(fixture: &str, query: &str) -> Result<(String, Value), Box<dyn Error>> {
    let query_plan = build_query_plan_with_defaults(
        &format!("fixture/tests/{fixture}.supergraph.graphql"),
        parse_operation(query),
    )?;
    Ok((format!("{query_plan}"), serde_json::to_value(&query_plan)?))
}

fn parent_count(path: &[Value]) -> usize {
    path.iter().filter(|e| *e == "Parent").count()
}

/// Port of `inc_from_context_multi_hop_ancestor`: the context is two entity hops above the
/// contextual field, all in one subgraph, so the rewrite walks up at least two parents.
#[test]
fn multi_hop_ancestor_through_entities() -> Result<(), Box<dyn Error>> {
    let (text, json) = plan(
        "context-apollo-multi-hop-entities",
        "{ a { b { c { value } } } }",
    )?;
    insta::assert_snapshot!(text);

    assert_eq!(fetch_count(&json), 2, "Apollo plans two fetches:\n{text}");
    let rewrites = context_rewrite_paths(&json);
    assert_eq!(rewrites.len(), 1, "{text}");
    assert!(
        parent_count(&rewrites[0]) >= 2,
        "a multi-hop context walks up at least two parents: {:?}",
        rewrites[0]
    );
    assert_eq!(
        rewrites[0].last(),
        Some(&serde_json::json!({"Key": "prop"}))
    );
    Ok(())
}

/// Port of `inc_from_context_self_context_reads_from_ancestor`: `Tree` sets the context and
/// also declares the contextual field, so `child.value` reads `tree.prop`, its ancestor's,
/// never its own.
#[test]
fn self_context_reads_from_ancestor() -> Result<(), Box<dyn Error>> {
    let (text, json) = plan(
        "context-apollo-self-context",
        "{ start { tree { child { value } } } }",
    )?;
    insta::assert_snapshot!(text);

    assert_eq!(fetch_count(&json), 2, "{text}");
    assert_eq!(
        context_rewrite_paths(&json),
        vec![vec![
            serde_json::json!("Parent"),
            serde_json::json!({"TypenameEquals": ["Tree"]}),
            serde_json::json!({"Key": "prop"}),
        ]],
        "Apollo's rewrite is [\"..\", \"... on Tree\", \"prop\"]:\n{text}"
    );
    Ok(())
}

/// Port of `inc_from_context_boundary_entity_with_self_context_reads_from_ancestor`: the same
/// self-context, reached through a key hop into the context subgraph. The boundary type is not
/// already the provider; `tree.prop` comes from the hop's own fetch.
#[test]
fn self_context_reached_through_an_entity_hop() -> Result<(), Box<dyn Error>> {
    let (text, json) = plan(
        "context-apollo-self-context-boundary",
        "{ start { tree { child { value } } } }",
    )?;
    insta::assert_snapshot!(text);

    assert_eq!(fetch_count(&json), 3, "Apollo plans three fetches:\n{text}");
    assert_eq!(
        context_rewrite_paths(&json),
        vec![vec![
            serde_json::json!("Parent"),
            serde_json::json!({"TypenameEquals": ["Tree"]}),
            serde_json::json!({"Key": "prop"}),
        ]],
        "{text}"
    );
    Ok(())
}

/// Port of `inc_from_context_type_conditioned_selection_plans`: `A` and `B` both set the
/// context and the selection has a branch for each. Entered through a key hop on `A`, the
/// rewrite keeps only the `... on A` branch, the one runtime type that can be the ancestor.
#[test]
fn type_conditioned_selection_entered_through_a_key_hop() -> Result<(), Box<dyn Error>> {
    let (text, json) = plan(
        "context-apollo-type-conditioned-key-hop",
        "{ a { child { value } } }",
    )?;
    insta::assert_snapshot!(text);

    assert_eq!(fetch_count(&json), 3, "Apollo plans three fetches:\n{text}");
    let rewrites = context_rewrite_paths(&json);
    assert_eq!(rewrites.len(), 1, "{text}");
    let rendered = serde_json::to_string(&rewrites[0])?;
    assert!(rendered.contains(r#""TypenameEquals":["A"]"#), "{rendered}");
    assert!(
        !rendered.contains(r#""B""#),
        "the B branch cannot apply here: {rendered}"
    );
    Ok(())
}

/// Port of Apollo's interface-entity context case (`fromContext with type conditions
/// interface` in query-planner-js, `set_context_test_impacts_on_query_planning` in the router):
/// the context is set on an interface that is itself an entity, so the rewrite must accept
/// either implementation as the ancestor.
#[test]
fn context_on_an_interface_that_is_an_entity() -> Result<(), Box<dyn Error>> {
    let (text, json) = plan(
        "context-apollo-interface-entity",
        "{ items { child { value } } }",
    )?;
    insta::assert_snapshot!(text);

    // Apollo emits one rewrite per implementation (`... on A`, `... on B`). Hive emits one
    // rewrite guarded by the interface, and its executor matches any implementation, which
    // `e2e/src/context.rs` `interface_entity_context_reads_each_implementation` asserts at
    // run time. Either shape is correct; what must hold is that the guard covers both.
    let rendered = serde_json::to_string(&context_rewrite_paths(&json))?;
    let per_implementation = rendered.contains(r#""A""#) && rendered.contains(r#""B""#);
    let interface_guard = rendered.contains(r#"{"TypenameEquals":["I"]}"#);
    assert!(per_implementation || interface_guard, "{rendered}");
    assert!(rendered.contains(r#"{"Key":"prop"}"#), "{rendered}");
    Ok(())
}
