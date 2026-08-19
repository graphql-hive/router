//! Building `representations` for an entity fetch.
//!
//! Today every entity is walked twice over the `requires` selection set — once by
//! `Value::to_hash` for dedup, once by `project_requires` to write the JSON — each with a
//! `binary_search` per field. This bench is the before/after for collapsing that into a
//! single walk that hashes the projected bytes.

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use graphql_tools::parser::query::Definition;
use hive_router_plan_executor::{
    introspection::schema::{PossibleTypes, SchemaWithMetadata},
    projection::request::push_representation,
    response::{subgraph_response::SubgraphResponse, value::Value},
};
use hive_router_query_planner::{
    ast::selection_set::SelectionSet,
    consumer_schema::ConsumerSchema,
    planner::{
        merged_shape::response_shape_for_selections,
        response_shape::ResponseShape,
        slot_path::{compile_requires, RequiresStep},
    },
    utils::parsing::{parse_operation, parse_schema},
};
use std::hint::black_box;

use ahash::HashMap as AHashMap;
use ahash::HashMapExt;

const SDL: &str = r#"
    type Query { users: [User!]! }
    type User { id: ID! tenantId: ID! region: String! name: String! }
"#;

/// `requires` selection sets are plain `SelectionSet`s, so parse one out of an operation.
fn requires_selection(source: &str) -> SelectionSet {
    let mut doc = parse_operation(source);
    let op = doc
        .definitions
        .iter_mut()
        .find_map(|def| match def {
            Definition::Operation(op) => Some(op),
            _ => None,
        })
        .expect("operation");
    op.selection_set().clone().into()
}

fn entities_payload(count: usize, distinct: usize) -> String {
    let mut out = String::from(r#"{"data":{"users":["#);
    for i in 0..count {
        if i > 0 {
            out.push(',');
        }
        let k = i % distinct;
        out.push_str(&format!(
            r#"{{"__typename":"User","id":"user-{k}","tenantId":"tenant-{k}","region":"eu-west-{k}","name":"Name {k}"}}"#
        ));
    }
    out.push_str("]}}");
    out
}

/// Mirrors the dedup loop in `Executor::prepare_job_future`.
fn build_representations(
    entities: &[Value<'_>],
    requires: &[RequiresStep],
    possible_types: &PossibleTypes,
) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(4096);
    buffer.push(b'[');
    let mut seen: AHashMap<u64, usize> = AHashMap::new();
    let mut next_index = 0usize;
    let arena = bumpalo::Bump::new();

    for entity in entities {
        push_representation(
            possible_types,
            requires,
            &[],
            &arena,
            entity,
            &mut buffer,
            &mut seen,
            &mut next_index,
        );
    }

    buffer.push(b']');
    buffer
}

fn requires_benches(c: &mut Criterion) {
    let supergraph = parse_schema(SDL);
    let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
    let schema_metadata = consumer_schema.schema_metadata();
    let possible_types = &schema_metadata.possible_types;

    // The entity position's shape, and the `requires` selection compiled against it.
    let entity_selections = requires_selection("{ __typename id tenantId region name }");
    let entity_shape: ResponseShape = response_shape_for_selections(&entity_selections);
    let requires = compile_requires(&requires_selection("{ id tenantId }"), &entity_shape);
    let users_shape = ResponseShape {
        fields: vec![
            hive_router_query_planner::planner::response_shape::ResponseShapeField {
                key: "users".to_string(),
                shape: entity_shape.clone(),
            },
        ],
        raw: false,
        inert: false,
        list_len_hint: Default::default(),
    };

    let mut group = c.benchmark_group("requires");

    for (count, distinct, label) in [
        (100usize, 100usize, "100_unique"),
        (1000, 1000, "1000_unique"),
        (1000, 500, "1000_half_dupes"),
        (1000, 50, "1000_mostly_dupes"),
        (1000, 10, "1000_almost_all_dupes"),
    ] {
        let response = SubgraphResponse::deserialize_from_bytes(
            Bytes::from(entities_payload(count, distinct)),
            Some(&users_shape),
        )
        .unwrap();
        let entities = match response.data.slot(0) {
            Some(Value::Array(items)) => items.clone(),
            other => panic!("expected users array, got {other:?}"),
        };

        group.bench_function(format!("hash_and_project/{label}"), |b| {
            b.iter_batched(
                || (),
                |_| {
                    let out = build_representations(&entities, &requires, possible_types);
                    black_box(out);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, requires_benches);
criterion_main!(benches);
