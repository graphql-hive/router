use bytes::Bytes;
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
use std::{env, hint::black_box, path::PathBuf, time::Instant};

#[derive(Clone, Copy)]
enum Mode {
    Deserialize,
    Project,
    Full,
}

struct Args {
    schema: PathBuf,
    query: PathBuf,
    payload: PathBuf,
    mode: Mode,
    iterations: usize,
}

struct Fixture {
    payload: Bytes,
    custom_scalar_paths: Option<CustomScalarPaths>,
    operation_type_name: &'static str,
    projection_plan: Vec<FieldProjectionPlan>,
    schema_metadata: &'static hive_router_plan_executor::introspection::schema::SchemaMetadata,
    projected_response_size_estimate: usize,
}

impl Fixture {
    fn deserialize(&self) -> SubgraphResponse<'static> {
        SubgraphResponse::deserialize_from_bytes(
            self.payload.clone(),
            self.custom_scalar_paths.as_ref(),
        )
        .expect("failed to deserialize CT payload")
    }
}

fn main() {
    let args = parse_args();
    let fixture = load_fixture(&args);
    let started = Instant::now();
    let mut total_output_bytes = 0usize;

    match args.mode {
        Mode::Deserialize => {
            for _ in 0..args.iterations {
                let response = fixture.deserialize();
                black_box(response);
            }
        }
        Mode::Project => {
            let response = fixture.deserialize();
            for _ in 0..args.iterations {
                let projected = project(&fixture, &response);
                total_output_bytes += projected.len();
                black_box(projected);
            }
            black_box(response);
        }
        Mode::Full => {
            for _ in 0..args.iterations {
                let response = fixture.deserialize();
                let projected = project(&fixture, &response);
                total_output_bytes += projected.len();
                black_box(projected);
                black_box(response);
            }
        }
    }

    let elapsed = started.elapsed();
    println!(
        "mode={} iterations={} input_bytes={} output_bytes={} elapsed_ms={:.3}",
        mode_name(args.mode),
        args.iterations,
        fixture.payload.len(),
        total_output_bytes,
        elapsed.as_secs_f64() * 1000.0,
    );
}

fn project(fixture: &Fixture, response: &SubgraphResponse<'static>) -> Vec<u8> {
    project_by_operation(
        black_box(&response.data),
        vec![],
        &ExecutionResultExtensions::default(),
        black_box(fixture.operation_type_name),
        black_box(&fixture.projection_plan),
        &None,
        fixture.projected_response_size_estimate,
        fixture.schema_metadata,
    )
    .expect("failed to project CT payload")
}

fn load_fixture(args: &Args) -> Fixture {
    let schema_sdl = std::fs::read_to_string(&args.schema)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", args.schema.display()));
    let query = std::fs::read_to_string(&args.query)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", args.query.display()));
    let payload = std::fs::read(&args.payload)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", args.payload.display()));

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

    Fixture {
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

fn parse_args() -> Args {
    let mut schema = workspace_file("CT_SCHEMA_PATH", "ct-super.graphql");
    let mut query = workspace_file("CT_QUERY_PATH", "ct-query.graphql");
    let mut payload = workspace_file("CT_PAYLOAD_PATH", "ct-payload.json");
    let mut mode = Mode::Full;
    let mut iterations = 100;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--schema" => schema = PathBuf::from(next_arg(&mut args, "--schema")),
            "--query" => query = PathBuf::from(next_arg(&mut args, "--query")),
            "--payload" => payload = PathBuf::from(next_arg(&mut args, "--payload")),
            "--mode" => mode = parse_mode(&next_arg(&mut args, "--mode")),
            "--iterations" | "-n" => {
                iterations = next_arg(&mut args, &arg)
                    .parse()
                    .expect("--iterations must be a positive integer")
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => panic!("unknown argument: {arg}"),
        }
    }

    Args {
        schema,
        query,
        payload,
        mode,
        iterations,
    }
}

fn workspace_file(env_name: &str, default_name: &str) -> PathBuf {
    env::var_os(env_name).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(default_name)
    })
}

fn next_arg(args: &mut impl Iterator<Item = String>, flag: &str) -> String {
    args.next()
        .unwrap_or_else(|| panic!("missing value for {flag}"))
}

fn parse_mode(value: &str) -> Mode {
    match value {
        "deserialize" => Mode::Deserialize,
        "project" => Mode::Project,
        "full" => Mode::Full,
        _ => panic!("unknown mode: {value}; expected deserialize, project, or full"),
    }
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Deserialize => "deserialize",
        Mode::Project => "project",
        Mode::Full => "full",
    }
}

fn print_help() {
    println!(
        "Usage: cargo run -p hive-router-plan-executor --profile profiling --example ct_response_profile -- [options]\n\n\
Options:\n\
  --schema <path>       Supergraph schema path. Default: CT_SCHEMA_PATH or ../../ct-super.graphql\n\
  --query <path>        Operation path. Default: CT_QUERY_PATH or ../../ct-query.graphql\n\
  --payload <path>      JSON response payload path. Default: CT_PAYLOAD_PATH or ../../ct-payload.json\n\
  --mode <mode>         deserialize, project, or full. Default: full\n\
  -n, --iterations <n>  Iterations. Default: 100"
    );
}
