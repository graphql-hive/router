//! The response pipeline end to end, in process: the real `execute_query_plan` with subgraph
//! executors that serve recorded bodies. Parsing, merging, representations, request bodies,
//! projection and teardown all run; the network doesn't.
//!
//! ```text
//! cargo bench -p hive-router --bench pipeline_benches -- [filter] [--iters N] [--bless]
//! ```
//!
//! Workloads:
//! - `bench_op`: `bench/operation.graphql` on `bench/supergraph.graphql`
//! - `search`: issue 1309's `listings` with the pet interface and `@requires`
//! - `ct`: one 3 MB root fetch, read from `$CT_DIR` (`ct-super.graphql`, `ct-query.graphql`,
//!   `ct-payload.json`). Skipped when the files aren't there, they aren't in git.
//!
//! Subgraph responses are recorded at startup from the in-process bench subgraphs (they are
//! deterministic). The client output and every subgraph request body are checked against the
//! references in `benches/fixtures/pipeline/<workload>/` (for ct, `$CT_DIR/pipeline-ref/`)
//! before anything is timed, JSON-equal: same keys in the same order, numbers by value.
//! `--bless` writes the references instead.
//!
//! Per request it prints thread CPU time (median, p10, p90), allocations and peak bytes over
//! the live heap at the start of the request. Teardown is inside the measured region.

use std::alloc::{GlobalAlloc, Layout};
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::stream::BoxStream;
use hive_router::executor::execution::client_request_details::{
    ClientRequestDetails, JwtRequestDetails, OperationDetails,
};
use hive_router::executor::execution::operation_name::OperationNameFactory;
use hive_router::executor::execution::plan::{
    execute_query_plan, CoerceVariablesPayload, ExecutionResultExtensions, QueryPlanExecutionOpts,
    QueryPlanExecutionResult,
};
use hive_router::executor::executors::common::{SubgraphExecutionRequest, SubgraphExecutor};
use hive_router::executor::executors::error::SubgraphExecutorError;
use hive_router::executor::executors::http::build_request_body;
use hive_router::executor::executors::map::SubgraphExecutorMap;
use hive_router::executor::introspection::partition::partition_operation;
use hive_router::executor::introspection::resolve::IntrospectionContext;
use hive_router::executor::plugin_context::PluginRequestState;
use hive_router::executor::plugins::hooks::on_supergraph_load::Supergraph;
use hive_router::executor::projection::plan::ProjectionPlan;
use hive_router::executor::response::subgraph_response::SubgraphResponse;
use hive_router::executor::slots::SlotPlanCell;
use hive_router::query_planner::ast::normalization::normalize_operation;
use hive_router::query_planner::ast::operation::OperationDefinition;
use hive_router::query_planner::planner::plan_nodes::QueryPlan;
use hive_router::query_planner::state::supergraph_state::OperationKind;
use hive_router::query_planner::utils::cancellation::CancellationToken;
use hive_router::query_planner::utils::parsing::parse_operation;
use hive_router::telemetry::traces::spans::graphql::GraphQLOperationSpan;
use hive_router::RouterGlobalAllocator;
use http::{StatusCode, Uri};

// ---------------------------------------------------------------------------------------------
// Counting allocator: allocations and peak live bytes, while `COUNTING` is on.

struct Counting;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static GLOBAL: Counting = Counting;

#[inline]
fn grow(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    if COUNTING.load(Ordering::Relaxed) {
        PEAK.fetch_max(live, Ordering::Relaxed);
    }
}

/// `--trace-allocs`: a backtrace of every allocation of one warm request
static TRACING: AtomicBool = AtomicBool::new(false);
static TRACES: Mutex<Vec<std::backtrace::Backtrace>> = Mutex::new(Vec::new());

thread_local! {
    static IN_TRACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn trace() {
    if !TRACING.load(Ordering::Relaxed) || IN_TRACE.with(|t| t.replace(true)) {
        return;
    }
    let backtrace = std::backtrace::Backtrace::force_capture();
    TRACES.lock().unwrap().push(backtrace);
    IN_TRACE.with(|t| t.set(false));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            trace();
        }
        grow(layout.size());
        RouterGlobalAllocator.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            trace();
        }
        grow(layout.size());
        RouterGlobalAllocator.alloc_zeroed(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        RouterGlobalAllocator.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // a realloc is an allocation as far as the count goes, it can move and copy
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            trace();
        }
        if new_size >= layout.size() {
            grow(new_size - layout.size());
        } else {
            LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
        }
        RouterGlobalAllocator.realloc(ptr, layout, new_size)
    }
}

// ---------------------------------------------------------------------------------------------
// Thread CPU time, steadier than wall time on a laptop.

#[repr(C)]
struct Timespec {
    sec: i64,
    nsec: i64,
}

extern "C" {
    fn clock_gettime(clock: u32, tp: *mut Timespec) -> i32;
}

#[cfg(target_os = "macos")]
const CLOCK_THREAD_CPUTIME_ID: u32 = 16;
#[cfg(not(target_os = "macos"))]
const CLOCK_THREAD_CPUTIME_ID: u32 = 3;

fn thread_cpu_ns() -> u64 {
    let mut ts = Timespec { sec: 0, nsec: 0 };
    unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    ts.sec as u64 * 1_000_000_000 + ts.nsec as u64
}

// ---------------------------------------------------------------------------------------------
// Recorded subgraphs.

/// Recorded bodies by subgraph and query, served in the order they were recorded.
#[derive(Default)]
struct Recording {
    bodies: HashMap<u64, (Vec<Bytes>, AtomicUsize)>,
}

/// Subgraph request bodies, by subgraph, in the order they were sent
type Requests = Vec<(String, Vec<u8>)>;

#[derive(Default)]
struct Shared {
    recording: Mutex<Recording>,
    /// Set while recording: where the live subgraphs are.
    live: Mutex<Option<(String, HashMap<String, String>)>>,
    /// Set while checking: every request body, in order.
    captured: Mutex<Option<Requests>>,
    /// Workloads whose subgraphs answer with one body whatever the request
    fixed: Mutex<Option<Bytes>>,
}

fn key(subgraph: &str, query: &str) -> u64 {
    let mut hasher = ahash::RandomState::with_seeds(1, 2, 3, 4).build_hasher();
    subgraph.hash(&mut hasher);
    query.hash(&mut hasher);
    hasher.finish()
}

impl Shared {
    fn rewind(&self) {
        let recording = self.recording.lock().unwrap();
        for (_, next) in recording.bodies.values() {
            next.store(0, Ordering::Relaxed);
        }
    }
}

struct Replay {
    subgraph: String,
    endpoint: Uri,
    shared: Arc<Shared>,
}

#[hive_router::async_trait]
impl SubgraphExecutor for Replay {
    fn executor_name(&self) -> &str {
        "replay"
    }

    fn endpoint(&self) -> &Uri {
        &self.endpoint
    }

    async fn execute<'a>(
        &self,
        request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
        _plugin_req_state: Option<&'a PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        // the http executor builds the body too, and the representations are in it
        let body = build_request_body(&request)?;
        if let Some(captured) = self.shared.captured.lock().unwrap().as_mut() {
            captured.push((self.subgraph.clone(), body.clone()));
        }

        let key = key(&self.subgraph, request.query);
        let live = self.shared.live.lock().unwrap().clone();
        let fixed = self.shared.fixed.lock().unwrap().clone();
        let bytes = if let Some(fixed) = fixed {
            fixed
        } else if let Some((origin, paths)) = live {
            let url = format!("{origin}{}", paths[&self.subgraph]);
            let response = reqwest::Client::new()
                .post(url)
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
                .expect("the bench subgraphs answer");
            let bytes = response.bytes().await.expect("a body");
            let mut recording = self.shared.recording.lock().unwrap();
            recording
                .bodies
                .entry(key)
                .or_default()
                .0
                .push(bytes.clone());
            bytes
        } else {
            let recording = self.shared.recording.lock().unwrap();
            let (bodies, next) = recording
                .bodies
                .get(&key)
                .unwrap_or_else(|| panic!("no recording for {}: {}", self.subgraph, request.query));
            let index = next.fetch_add(1, Ordering::Relaxed);
            bodies
                .get(index)
                .unwrap_or_else(|| panic!("{} called more often than recorded", self.subgraph))
                .clone()
        };

        // like the http executor: the slot pipeline reads the body itself
        if request.raw_response {
            return Ok(SubgraphResponse {
                bytes: Some(bytes),
                status: Some(StatusCode::OK),
                ..Default::default()
            });
        }
        let response = match request.response_shape {
            Some(shape) => {
                SubgraphResponse::deserialize_with_shape(bytes, shape, request.custom_scalar_paths)
            }
            None => SubgraphResponse::deserialize_from_bytes(bytes, request.custom_scalar_paths),
        };
        response.map(|mut response| {
            response.status = Some(StatusCode::OK);
            response
        })
    }

    async fn subscribe<'a>(
        &self,
        _request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
    ) -> Result<
        BoxStream<'static, Result<SubgraphResponse<'static>, SubgraphExecutorError>>,
        SubgraphExecutorError,
    > {
        unimplemented!("no subscriptions in the pipeline bench")
    }
}

// ---------------------------------------------------------------------------------------------
// Workloads.

struct Workload {
    name: &'static str,
    references: PathBuf,
    // kept alive, the plan's metadata lives in it
    supergraph: Supergraph,
    plan: QueryPlan,
    projection_plan: Arc<ProjectionPlan>,
    root_type_name: String,
    operation_for_plan: Arc<OperationDefinition>,
    executors: Arc<SubgraphExecutorMap>,
    shared: Arc<Shared>,
    client_request: Arc<ClientRequestDetails<'static>>,
    introspection: Arc<IntrospectionContext>,
    variables: Arc<CoerceVariablesPayload>,
    slot_plan: Arc<SlotPlanCell>,
}

fn workload(
    name: &'static str,
    supergraph_sdl: &str,
    operation: &str,
    references: PathBuf,
) -> Workload {
    let supergraph = Supergraph::from_sdl(name, supergraph_sdl, Default::default())
        .expect("the supergraph builds");
    let planner = &supergraph.planner;
    let metadata = Arc::clone(&supergraph.metadata);

    let document = parse_operation(operation);
    let normalized = normalize_operation(&planner.supergraph, &document, None)
        .expect("the operation normalizes");
    let operation = normalized.operation;
    let (root_type_name, projection_plan) = ProjectionPlan::from_operation(&operation, &metadata);
    let root_type_name = root_type_name.to_string();
    let partitioned = partition_operation(operation);
    assert!(partitioned.introspection_operation.is_none());
    let operation_for_plan = Arc::new(partitioned.downstream_operation);
    let plan = planner
        .plan_from_normalized_operation(
            &operation_for_plan,
            Default::default(),
            &CancellationToken::new(),
        )
        .expect("the operation plans")
        .into_executable(&planner.supergraph);

    let endpoints = planner.supergraph.subgraph_endpoint_map.clone();
    let executors = SubgraphExecutorMap::from_http_endpoint_map(
        &endpoints,
        Default::default(),
        Default::default(),
        Default::default(),
        None,
        Arc::new(
            hive_router::telemetry::TelemetryContext::from_propagation_config(
                &Default::default(),
                &Default::default(),
            ),
        ),
        Arc::new(hive_router::DashMap::new()),
    )
    .expect("the executor map builds");
    let shared = Arc::new(Shared::default());
    for (subgraph, endpoint) in &endpoints {
        let replay = Replay {
            subgraph: subgraph.clone(),
            endpoint: endpoint.parse().expect("a valid endpoint"),
            shared: Arc::clone(&shared),
        };
        executors.replace_http_executor(subgraph, replay.to_boxed_arc());
    }

    let method: &'static http::Method = Box::leak(Box::new(http::Method::POST));
    let url: &'static Uri = Box::leak(Box::new("http://localhost/graphql".parse().unwrap()));
    let query: &'static str = Box::leak(operation_text(&document).into_boxed_str());
    let client_request = Arc::new(ClientRequestDetails {
        method,
        url,
        headers: Default::default(),
        operation: OperationDetails {
            name: None,
            query,
            kind: "query",
        },
        jwt: JwtRequestDetails::Unauthenticated.into(),
        path_params: Default::default(),
    });
    let variables = Arc::new(CoerceVariablesPayload {
        variables_map: None,
    });
    let introspection = Arc::new(IntrospectionContext {
        query: None,
        schema: Arc::clone(&planner.consumer_schema.document),
        metadata,
        variables: Arc::clone(&variables),
    });

    Workload {
        name,
        references,
        supergraph,
        plan,
        projection_plan: Arc::new(projection_plan),
        root_type_name,
        operation_for_plan,
        executors: Arc::new(executors),
        shared,
        client_request,
        introspection,
        variables,
        slot_plan: SlotPlanCell::new(),
    }
}

fn operation_text(document: &graphql_tools::parser::query::Document<'static, String>) -> String {
    document.to_string()
}

impl Workload {
    async fn run(&self) -> Vec<u8> {
        self.shared.rewind();
        let opts = QueryPlanExecutionOpts {
            query_plan: &self.plan,
            operation_for_plan: Arc::clone(&self.operation_for_plan),
            projection_plan: Arc::clone(&self.projection_plan),
            headers_plan: Default::default(),
            extensions_plan: Default::default(),
            variable_values: Arc::clone(&self.variables),
            extensions: ExecutionResultExtensions::default(),
            client_request: Arc::clone(&self.client_request),
            introspection_context: Arc::clone(&self.introspection),
            operation_type_name: self.root_type_name.clone(),
            operation_kind: OperationKind::Query,
            executors: Arc::clone(&self.executors),
            jwt_auth_forwarding: None,
            graphql_error_recorder: None,
            demand_control_context: None,
            initial_errors: Vec::new(),
            span: GraphQLOperationSpan {
                span: tracing::Span::none(),
            },
            plugin_req_state: None,
            operation_name_factory: OperationNameFactory::default(),
            response_header_sink: Default::default(),
            error_masking_runtime: Default::default(),
            connection_fingerprint: None,
            slot_plan: Some(Arc::clone(&self.slot_plan)),
        };
        match execute_query_plan(opts).await.expect("the plan executes") {
            QueryPlanExecutionResult::Single(output) => output.body,
            QueryPlanExecutionResult::Stream(_) => panic!("not a subscription"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// JSON-equal: same keys in the same order, numbers by value, strings by decoded value.

fn json_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| json_equal(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((ka, va), (kb, vb))| ka == kb && json_equal(va, vb))
        }
        (a, b) => a == b,
    }
}

fn parse_json(bytes: &[u8], what: &str) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap_or_else(|err| {
        panic!(
            "{what} is not JSON ({err}): {}",
            String::from_utf8_lossy(&bytes[..bytes.len().min(300)])
        )
    })
}

fn check_or_bless(workload: &Workload, output: &[u8], requests: &[(String, Vec<u8>)], bless: bool) {
    let dir = &workload.references;
    let output_path = dir.join("output.json");
    let requests_path = dir.join("requests.json");
    let requests_json = serde_json::Value::Array(
        requests
            .iter()
            .map(|(subgraph, body)| {
                serde_json::json!({
                    "subgraph": subgraph,
                    "body": parse_json(body, "a subgraph request body"),
                })
            })
            .collect(),
    );

    if bless {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(&output_path, output).unwrap();
        std::fs::write(
            &requests_path,
            serde_json::to_vec_pretty(&requests_json).unwrap(),
        )
        .unwrap();
        println!("{}: blessed {}", workload.name, dir.display());
        return;
    }

    let expected_output = std::fs::read(&output_path).unwrap_or_else(|_| {
        panic!(
            "no reference at {}, run with --bless on the baseline",
            output_path.display()
        )
    });
    let expected = parse_json(&expected_output, "the reference output");
    let actual = parse_json(output, "the client output");
    if !json_equal(&expected, &actual) {
        let actual_path = dir.join("output.actual.json");
        std::fs::write(&actual_path, output).unwrap();
        panic!(
            "{}: client output differs from the reference ({} vs {} bytes), see {}",
            workload.name,
            output.len(),
            expected_output.len(),
            actual_path.display()
        );
    }
    let expected_requests = parse_json(&std::fs::read(&requests_path).unwrap(), "requests.json");
    if !json_equal(&expected_requests, &requests_json) {
        let actual_path = dir.join("requests.actual.json");
        std::fs::write(
            &actual_path,
            serde_json::to_vec_pretty(&requests_json).unwrap(),
        )
        .unwrap();
        panic!(
            "{}: subgraph request bodies differ from the reference, see {}",
            workload.name,
            actual_path.display()
        );
    }
    println!(
        "{}: output ({} bytes) and {} request bodies match the reference",
        workload.name,
        output.len(),
        requests.len()
    );
}

// ---------------------------------------------------------------------------------------------

struct Stats {
    cpu_ns: Vec<u64>,
    allocs: usize,
    peak: usize,
}

fn percentile(sorted: &[u64], p: f64) -> f64 {
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index] as f64 / 1000.0
}

fn measure(runtime: &tokio::runtime::Runtime, workload: &Workload, iterations: usize) -> Stats {
    // warm up: caches, pools, the allocator's free lists
    let warmup = (iterations / 10).max(3);
    for _ in 0..warmup {
        std::hint::black_box(runtime.block_on(workload.run()));
    }

    // allocations and peak from one warm request, they don't vary between requests
    ALLOCS.store(0, Ordering::Relaxed);
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    std::hint::black_box(runtime.block_on(workload.run()));
    COUNTING.store(false, Ordering::Relaxed);
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(base);

    if std::env::args().any(|arg| arg == "--trace-allocs") {
        TRACES.lock().unwrap().clear();
        COUNTING.store(true, Ordering::Relaxed);
        TRACING.store(true, Ordering::Relaxed);
        std::hint::black_box(runtime.block_on(workload.run()));
        TRACING.store(false, Ordering::Relaxed);
        COUNTING.store(false, Ordering::Relaxed);
        print_traces(workload.name);
    }

    let mut cpu_ns = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = thread_cpu_ns();
        // the output is dropped inside the measured region, like the rest of the request
        drop(std::hint::black_box(runtime.block_on(workload.run())));
        cpu_ns.push(thread_cpu_ns() - start);
    }
    cpu_ns.sort_unstable();
    Stats {
        cpu_ns,
        allocs,
        peak,
    }
}

fn main() {
    tracing::subscriber::set_global_default(tracing::subscriber::NoSubscriber::default())
        .expect("no subscriber yet");
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let bless = args.iter().any(|arg| arg == "--bless");
    let iterations = args
        .iter()
        .position(|arg| arg == "--iters")
        .and_then(|index| args.get(index + 1))
        .map(|value| value.parse().expect("--iters takes a number"))
        .unwrap_or(500usize);
    let filters: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(index, arg)| {
            !arg.starts_with("--") && (*index == 0 || args[index - 1] != "--iters")
        })
        .map(|(_, arg)| arg)
        .collect();
    let selected =
        |name: &str| filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str()));

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/pipeline");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    // the bench subgraphs, in process, to record from
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let _server = runtime.block_on(async {
        std::env::set_var("HOST", "127.0.0.1");
        let server = subgraphs::start_subgraphs_server(Some(port));
        tokio::time::sleep(Duration::from_millis(200)).await;
        server
    });
    let origin = format!("http://127.0.0.1:{port}");

    let mut workloads = Vec::new();
    if selected("bench_op") {
        workloads.push(workload(
            "bench_op",
            &std::fs::read_to_string(root.join("bench/supergraph.graphql")).unwrap(),
            &std::fs::read_to_string(root.join("bench/operation.graphql")).unwrap(),
            fixtures.join("bench_op"),
        ));
    }
    if selected("search") {
        workloads.push(workload(
            "search",
            &std::fs::read_to_string(root.join("e2e/src/issues/supergraph.1309.graphql")).unwrap(),
            "{ listings { id rank pet { __typename id name ... on Cat { whiskers } ... on Dog { tricks } } } }",
            fixtures.join("search"),
        ));
    }
    let ct_dir = std::env::var("CT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap()).join("Work/guild/hive-router-backup")
        });
    if selected("ct") {
        match (
            std::fs::read_to_string(ct_dir.join("ct-super.graphql")),
            std::fs::read_to_string(ct_dir.join("ct-query.graphql")),
            std::fs::read(ct_dir.join("ct-payload.json")),
        ) {
            (Ok(sdl), Ok(query), Ok(payload)) => {
                let ct = workload("ct", &sdl, &query, ct_dir.join("pipeline-ref"));
                *ct.shared.fixed.lock().unwrap() = Some(Bytes::from(payload));
                workloads.push(ct);
            }
            _ => println!("ct: skipped, no fixtures in {}", ct_dir.display()),
        }
    }

    for workload in &workloads {
        let slots = hive_router::executor::slots::enabled()
            && workload
                .slot_plan
                .get(
                    &workload.plan,
                    &workload.projection_plan,
                    &workload.introspection.metadata,
                    &workload.root_type_name,
                )
                .is_some();
        println!(
            "{}: {} pipeline{}",
            workload.name,
            if slots { "slot" } else { "value" },
            hive_router::executor::slots::compile::SlotPlan::compile(
                &workload.plan,
                &workload.projection_plan,
                &workload.introspection.metadata,
                &workload.root_type_name,
            )
            .err()
            .map(|reason| format!(" (slots can't: {})", reason.0))
            .unwrap_or_default()
        );
        if std::env::var("PIPELINE_SHOW_PROJECTION").is_ok() {
            println!("{}", workload.projection_plan);
        }
        let fetches = count_fetches(&workload.plan);
        // record
        if workload.shared.fixed.lock().unwrap().is_none() {
            let paths = workload
                .supergraph
                .planner
                .supergraph
                .subgraph_endpoint_map
                .iter()
                .map(|(name, url)| {
                    let uri: Uri = url.parse().unwrap();
                    (name.clone(), uri.path().to_string())
                })
                .collect();
            *workload.shared.live.lock().unwrap() = Some((origin.clone(), paths));
            runtime.block_on(workload.run());
            *workload.shared.live.lock().unwrap() = None;
        }
        // check
        *workload.shared.captured.lock().unwrap() = Some(Vec::new());
        let output = runtime.block_on(workload.run());
        let requests = workload.shared.captured.lock().unwrap().take().unwrap();
        println!(
            "{}: {fetches} fetch nodes, {} subgraph requests",
            workload.name,
            requests.len()
        );
        check_or_bless(workload, &output, &requests, bless);
    }
    if bless {
        return;
    }

    println!();
    println!(
        "{:<10} {:>10} {:>10} {:>10} {:>8} {:>10}",
        "workload", "cpu p50µs", "p10", "p90", "allocs", "peak KB"
    );
    for workload in &workloads {
        let iterations = if workload.name == "ct" {
            (iterations / 5).max(20)
        } else {
            iterations
        };
        let stats = measure(&runtime, workload, iterations);
        println!(
            "{:<10} {:>10.1} {:>10.1} {:>10.1} {:>8} {:>10.1}",
            workload.name,
            percentile(&stats.cpu_ns, 0.5),
            percentile(&stats.cpu_ns, 0.1),
            percentile(&stats.cpu_ns, 0.9),
            stats.allocs,
            stats.peak as f64 / 1024.0
        );
    }
}

fn count_fetches(plan: &QueryPlan) -> usize {
    let text = format!("{plan}");
    text.matches("Fetch(").count()
}

/// Groups the traced allocations by the innermost two `hive_router` frames.
fn print_traces(workload: &str) {
    let traces = std::mem::take(&mut *TRACES.lock().unwrap());
    let mut groups: HashMap<String, usize> = HashMap::new();
    for trace in &traces {
        let text = format!("{trace}");
        let frames: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|line| line.contains("hive_router::") && !line.starts_with("at "))
            .filter(|line| !line.contains("RouterGlobalAllocator"))
            .take(2)
            .collect();
        let key = if frames.is_empty() {
            "(outside hive_router)".to_string()
        } else {
            frames
                .iter()
                .map(|frame| frame.split_once(": ").map_or(*frame, |(_, name)| name))
                .map(|name| name.rsplit_once("::h").map_or(name, |(name, _)| name))
                .collect::<Vec<_>>()
                .join("  <-  ")
        };
        *groups.entry(key).or_default() += 1;
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by_key(|group| std::cmp::Reverse(group.1));
    println!("{workload}: {} allocations by site", traces.len());
    for (key, count) in groups {
        println!("{count:>5}  {key}");
    }
}
