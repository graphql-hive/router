use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use hive_router_plan_executor::{
    execution::plan::ExecutionResultExtensions,
    introspection::schema::SchemaWithMetadata,
    projection::{plan::FieldProjectionPlan, response::project_by_operation},
    response::subgraph_response::SubgraphResponse,
};
use hive_router_query_planner::{
    ast::normalization::normalize_operation,
    graph::PlannerOverrideContext,
    planner::{
        plan_nodes::{CustomScalarPaths, PlanNode, QueryPlan},
        Planner,
    },
    utils::{
        cancellation::CancellationToken,
        parsing::{parse_operation, parse_schema},
    },
};
use pprof::criterion::{Output, PProfProfiler};
use std::{env, hint::black_box, path::PathBuf};

struct CtBenchFixture {
    payload: Bytes,
    custom_scalar_paths: Option<CustomScalarPaths>,
    operation_type_name: &'static str,
    projection_plan: Vec<FieldProjectionPlan>,
    schema_metadata: &'static hive_router_plan_executor::introspection::schema::SchemaMetadata,
    projected_response_size_estimate: usize,
}

impl CtBenchFixture {
    fn deserialize(&self) -> SubgraphResponse<'static> {
        SubgraphResponse::deserialize_from_bytes(
            self.payload.clone(),
            self.custom_scalar_paths.as_ref(),
        )
        .expect("failed to deserialize CT payload")
    }
}

fn workspace_file(env_name: &str, default_name: &str) -> PathBuf {
    env::var_os(env_name).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(default_name)
    })
}

fn load_fixture() -> CtBenchFixture {
    let schema_path = workspace_file("CT_SCHEMA_PATH", "ct-super.graphql");
    let query_path = workspace_file("CT_QUERY_PATH", "ct-query.graphql");
    let payload_path = workspace_file("CT_PAYLOAD_PATH", "ct-payload.json");

    let schema_sdl = std::fs::read_to_string(&schema_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", schema_path.display()));
    let query = std::fs::read_to_string(&query_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", query_path.display()));
    let payload = std::fs::read(&payload_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", payload_path.display()));

    let parsed_schema = parse_schema(&schema_sdl);
    let planner = Planner::new_from_supergraph(&parsed_schema, Default::default())
        .expect("failed to create planner");
    let parsed_operation = parse_operation(&query);
    let normalized_document = normalize_operation(&planner.supergraph, &parsed_operation, None)
        .expect("failed to normalize operation");
    let normalized_operation = normalized_document.executable_operation();
    let schema_metadata = Box::leak(Box::new(planner.consumer_schema.schema_metadata()));
    let (operation_type_name, projection_plan) =
        FieldProjectionPlan::from_operation(normalized_operation, schema_metadata);
    let query_plan = planner
        .plan_from_normalized_operation(
            normalized_operation,
            PlannerOverrideContext::default(),
            &CancellationToken::new(),
        )
        .expect("failed to build query plan");

    CtBenchFixture {
        projected_response_size_estimate: payload.len(),
        payload: Bytes::from(payload),
        custom_scalar_paths: first_custom_scalar_paths(&query_plan).cloned(),
        operation_type_name,
        projection_plan,
        schema_metadata,
    }
}

fn first_custom_scalar_paths(query_plan: &QueryPlan) -> Option<&CustomScalarPaths> {
    fn visit(node: &PlanNode) -> Option<&CustomScalarPaths> {
        match node {
            PlanNode::Fetch(fetch) => fetch.custom_scalar_paths.as_ref(),
            PlanNode::BatchFetch(fetch) => fetch.custom_scalar_paths.as_ref(),
            PlanNode::Sequence(sequence) => sequence.nodes.iter().find_map(visit),
            PlanNode::Parallel(parallel) => parallel.nodes.iter().find_map(visit),
            PlanNode::Flatten(flatten) => visit(&flatten.node),
            PlanNode::Condition(condition) => condition
                .if_clause
                .as_deref()
                .and_then(visit)
                .or_else(|| condition.else_clause.as_deref().and_then(visit)),
            PlanNode::Subscription(_) | PlanNode::Defer(_) => None,
        }
    }

    query_plan.node.as_ref().and_then(visit)
}

fn ct_response_benches(c: &mut Criterion) {
    let fixture = load_fixture();
    let mut group = c.benchmark_group("ct_response");
    group.throughput(Throughput::Bytes(fixture.payload.len() as u64));

    group.bench_function("deserialize_and_drop", |b| {
        b.iter(|| {
            let response = SubgraphResponse::deserialize_from_bytes(
                black_box(fixture.payload.clone()),
                fixture.custom_scalar_paths.as_ref(),
            )
            .expect("failed to deserialize CT payload");
            black_box(response);
        });
    });

    let projection_response = fixture.deserialize();
    group.bench_function("project_only", |b| {
        b.iter(|| {
            let projected = project_by_operation(
                black_box(&projection_response.data),
                vec![],
                &ExecutionResultExtensions::default(),
                black_box(fixture.operation_type_name),
                black_box(&fixture.projection_plan),
                &None,
                fixture.projected_response_size_estimate,
                fixture.schema_metadata,
            )
            .expect("failed to project CT payload");
            black_box(projected);
        });
    });

    group.bench_function("full_deserialize_project_drop", |b| {
        b.iter(|| {
            let response = SubgraphResponse::deserialize_from_bytes(
                black_box(fixture.payload.clone()),
                fixture.custom_scalar_paths.as_ref(),
            )
            .expect("failed to deserialize CT payload");
            let projected = project_by_operation(
                black_box(&response.data),
                vec![],
                &ExecutionResultExtensions::default(),
                black_box(fixture.operation_type_name),
                black_box(&fixture.projection_plan),
                &None,
                fixture.projected_response_size_estimate,
                fixture.schema_metadata,
            )
            .expect("failed to project CT payload");
            black_box(projected);
            black_box(response);
        });
    });

    group.bench_function("drop_value_tree", |b| {
        b.iter_batched(
            || fixture.deserialize(),
            |response| {
                black_box(response);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .with_profiler(PProfProfiler::new(1000, Output::Protobuf));
    targets = ct_response_benches
}
criterion_main!(benches);
