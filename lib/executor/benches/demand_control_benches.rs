use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use hive_router_plan_executor::execution::demand_control::{
    compile_actual_subgraph_cost_plan, estimate_actual_subgraph_response_cost_with_compiled_plan,
};
use hive_router_plan_executor::response::value::Value;
use hive_router_query_planner::ast::{
    document::Document, normalization::normalize_operation, operation::SubgraphFetchOperation,
};
use hive_router_query_planner::state::supergraph_state::SupergraphState;
use hive_router_query_planner::utils::parsing::{parse_operation, parse_schema};
use std::hint::black_box;

fn build_subgraph_operation(
    supergraph_sdl: &str,
    operation_str: &str,
) -> (SupergraphState, SubgraphFetchOperation) {
    let schema = parse_schema(supergraph_sdl);
    let supergraph_state = SupergraphState::new(&schema);
    let normalized =
        normalize_operation(&supergraph_state, &parse_operation(operation_str), None).unwrap();
    let document = Document {
        operation: normalized.operation.clone(),
        fragments: vec![],
    };
    let document_str = document.to_string();
    (
        supergraph_state,
        SubgraphFetchOperation {
            hash: normalized.operation.hash(),
            document,
            document_str,
        },
    )
}

fn demand_control_benchmarks(c: &mut Criterion) {
    // --- Nested lists with conditional field ---
    let (nested_supergraph_state, nested_operation) = build_subgraph_operation(
        r#"
        type Query {
            products: [Product!]!
        }
        type Product {
            id: ID!
            details: Details
        }
        type Details {
            sku: String
        }
        "#,
        r#"
        query($includeDetails: Boolean!) {
            products {
                id
                details @include(if: $includeDetails) {
                    sku
                }
            }
        }
        "#,
    );
    let nested_compiled_plan =
        compile_actual_subgraph_cost_plan(&nested_operation, &nested_supergraph_state);
    let nested_response: Value<'static> = sonic_rs::from_str(
        r#"{
            "products": [
                { "id": "p1", "details": { "sku": "sku-1" } },
                { "id": "p2", "details": null },
                { "id": "p3", "details": { "sku": "sku-3" } }
            ]
        }"#,
    )
    .unwrap();
    let nested_variable_values = Some(std::collections::HashMap::from([(
        "includeDetails".to_string(),
        sonic_rs::json!(true),
    )]));

    c.bench_function("demand_control/nested_lists/compiled", |b| {
        b.iter(|| {
            black_box(estimate_actual_subgraph_response_cost_with_compiled_plan(
                black_box(&nested_compiled_plan),
                black_box(&nested_response),
                black_box(&nested_variable_values),
            ))
        });
    });

    // --- Single _entities (FlattenFetch) ---
    let (entities_supergraph_state, entities_operation) = build_subgraph_operation(
        r#"
        scalar _Any
        union _Entity = Book | Author
        type Query {
            _entities(representations: [_Any!]!): [_Entity]!
        }
        type Book { title: String }
        type Author { name: String }
        "#,
        r#"
        query($representations: [_Any!]!) {
            _entities(representations: $representations) {
                __typename
                ... on Book { title }
                ... on Author { name }
            }
        }
        "#,
    );
    let entities_compiled_plan =
        compile_actual_subgraph_cost_plan(&entities_operation, &entities_supergraph_state);
    let entities_response: Value<'static> = sonic_rs::from_str(
        r#"{
            "_entities": [
                { "__typename": "Book", "title": "Book A" },
                { "__typename": "Author", "name": "Author B" },
                { "__typename": "Book", "title": "Book C" }
            ]
        }"#,
    )
    .unwrap();

    c.bench_function("demand_control/entities_flatten_fetch/compiled", |b| {
        b.iter(|| {
            black_box(estimate_actual_subgraph_response_cost_with_compiled_plan(
                black_box(&entities_compiled_plan),
                black_box(&entities_response),
                black_box(&None::<std::collections::HashMap<String, sonic_rs::Value>>),
            ))
        });
    });

    // --- Aliased _entities groups (BatchFetch) ---
    let batch_op_str = r#"
        query($r0:[_Any!]!,$r1:[_Any!]!){
            _e0:_entities(representations:$r0){
                __typename
                ... on Book { title }
            }
            _e1:_entities(representations:$r1){
                __typename
                ... on Author { name }
            }
        }
    "#;
    let schema = parse_schema(
        r#"
        scalar _Any
        union _Entity = Book | Author
        type Query {
            _entities(representations: [_Any!]!): [_Entity]!
        }
        type Book { title: String }
        type Author { name: String }
        "#,
    );
    let batch_supergraph_state = SupergraphState::new(&schema);
    let normalized = normalize_operation(
        &batch_supergraph_state,
        &parse_operation(batch_op_str),
        None,
    )
    .unwrap();
    let batch_doc = Document {
        operation: normalized.operation.clone(),
        fragments: vec![],
    };
    let batch_operation = SubgraphFetchOperation {
        hash: normalized.operation.hash(),
        document_str: batch_doc.to_string(),
        document: batch_doc,
    };
    let batch_compiled_plan =
        compile_actual_subgraph_cost_plan(&batch_operation, &batch_supergraph_state);
    let batch_response: Value<'static> = sonic_rs::from_str(
        r#"{
            "_e0": [
                { "__typename": "Book", "title": "Book A" },
                { "__typename": "Book", "title": "Book B" }
            ],
            "_e1": [
                { "__typename": "Author", "name": "Author X" }
            ]
        }"#,
    )
    .unwrap();

    c.bench_function("demand_control/entities_batch_fetch/compiled", |b| {
        b.iter(|| {
            black_box(estimate_actual_subgraph_response_cost_with_compiled_plan(
                black_box(&batch_compiled_plan),
                black_box(&batch_response),
                black_box(&None::<std::collections::HashMap<String, sonic_rs::Value>>),
            ))
        });
    });
}

criterion_group!(benches, demand_control_benchmarks);
criterion_main!(benches);
