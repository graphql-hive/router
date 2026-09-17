// Matches e2e/src/lib.rs. This target builds the same nested ntex middleware.
#![recursion_limit = "256"]

//! Measures retained heap per unique planned operation.
//! Covers parse, validate, normalize, plan caches, and other per-operation state.
//! Uses query-plan dry-run, so no subgraph server is needed.
//! Update `BASELINE_BYTES_PER_OPERATION` when an intentional change moves the result.
//!
use e2e::testkit::TestRouter;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Currently allocated heap bytes
static CURRENT_BYTES: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            CURRENT_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            CURRENT_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = System.realloc(ptr, layout, new_size);
        if !new_ptr.is_null() {
            // Only the difference moves, the original block's size was already counted.
            if new_size >= layout.size() {
                CURRENT_BYTES.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            } else {
                CURRENT_BYTES.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn current_bytes() -> usize {
    CURRENT_BYTES.load(Ordering::Relaxed)
}

const SUPERGRAPH_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../bin/router/fixture/grafbase-many-plans/supergraph.graphql"
);

/// Schema and operation carrying field arguments, an input object and a fragment, planned
/// with demand control on so the cost formula and the compiled actual-cost plan are cached too.
const COST_SUPERGRAPH_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/demand_control/custom_cost_schema.graphql"
);

const COST_OPERATION: &str =
    include_str!("../../fixtures/demand_control/custom_cost_query.graphql");

/// Enough entries for the estimate to settle; the operation is small, so this stays quick.
const MEASURED_COST_OPERATIONS: usize = 400;

/// Stays below cache capacity to avoid eviction
const MEASURED_OPERATIONS: usize = 800;

/// Warms up one-time allocations before measuring
const WARMUP_OPERATIONS: usize = 25;

/// Retained heap bytes per operation for `grafbase-many-plans`
const BASELINE_BYTES_PER_OPERATION: usize = 7_331;

/// Allowed increase above the baseline
const TOLERANCE: f64 = 0.05;

/// How far the weigher's estimate may sit from the heap the caches actually give back.
/// Allocator size classes, moka's own per-entry bookkeeping, and subtrees shared through an
/// `Arc` (counted once per cache that points at them) all keep this from landing on zero.
///
/// The corpora below sit at 2-5%, and charging a `BTreeMap` per entry instead of per node -
/// the one real bug this check has caught so far - reads 17%. Leave the room between those.
const ESTIMATE_TOLERANCE: f64 = 0.10;

/// Builds unique operations that also produce different query plans.
fn operation(index: usize) -> String {
    let a = index % 7;
    let b = (index / 7) % 7;
    let c = (index / 49) % 7;
    let d = (index / 343) % 7;
    format!(
        "query MemOp{index} {{ node {{ n{a} {{ n{b} {{ f{c} f{d} n{a} {{ f{b} f{c} }} }} }} }} }}"
    )
}

/// The custom-`@cost` fixture with a unique alias on its first field, so every request is a
/// distinct document in all four caches.
fn cost_operation(index: usize) -> String {
    // the fixture's operation is anonymous, and its brace is the only one alone on a line
    let aliased = COST_OPERATION.replacen("\n{\n", &format!("\n{{\n  a{index}: "), 1);
    assert_ne!(
        aliased, COST_OPERATION,
        "could not alias the fixture operation, so the requests would not be unique"
    );
    aliased
}

#[ntex::main]
async fn main() {
    let router = TestRouter::builder()
        .inline_config(format!(
            r#"supergraph:
  source: file
  path: "{SUPERGRAPH_PATH}"
query_planner:
  allow_expose: true
cache:
  router:
    parsing:
      max_size: 1GB
  supergraph:
    validation:
      max_size: 1GB
    normalization:
      max_size: 1GB
    query_plans:
      max_size: 1GB
"#
        ))
        .build()
        .start()
        .await;

    let dry_run = e2e::testkit::some_header_map!("hive-expose-query-plan" => "dry-run");

    for index in 0..WARMUP_OPERATIONS {
        let resp = router
            .send_graphql_request(&operation(usize::MAX - index), None, dry_run.clone())
            .await;
        assert!(resp.status().is_success(), "warmup request failed");
    }
    quiesce(&router).await;

    let before = current_bytes();

    for index in 0..MEASURED_OPERATIONS {
        let resp = router
            .send_graphql_request(&operation(index), None, dry_run.clone())
            .await;
        assert!(
            resp.status().is_success(),
            "operation {index} failed to plan"
        );
    }
    quiesce(&router).await;

    // Make sure the test actually populated the plan cache
    let cached_plans = plan_cache_entries(&router);
    assert!(
        cached_plans >= MEASURED_OPERATIONS as u64,
        "expected at least {MEASURED_OPERATIONS} cached plans, found {cached_plans}: \
         the operations were not unique, or planning never ran"
    );

    let after = current_bytes();
    let total = after.saturating_sub(before);
    let per_operation = total / MEASURED_OPERATIONS;

    println!(
        r#"{{"metric":"retained_memory","operations":{MEASURED_OPERATIONS},"total_bytes":{total},"bytes_per_operation":{per_operation}}}"#
    );

    check_weigher_against_reality(&router, "grafbase-many-plans", after).await;
    check_weigher_on_arguments_and_demand_control().await;

    let ceiling = (BASELINE_BYTES_PER_OPERATION as f64 * (1.0 + TOLERANCE)) as usize;
    if per_operation > ceiling {
        eprintln!(
            "retained memory regressed: {per_operation} bytes/operation exceeds the ceiling \
             of {ceiling} (baseline {BASELINE_BYTES_PER_OPERATION} + {:.0}% tolerance).\n\
             If this growth is intended, set BASELINE_BYTES_PER_OPERATION to {per_operation}.",
            TOLERANCE * 100.0
        );
        std::process::exit(1);
    }

    eprintln!("ok: {per_operation} bytes/operation, ceiling {ceiling}");
}

/// Runs the same check over operations the `grafbase-many-plans` corpus never produces:
/// argument maps, an input object, a fragment, and the demand-control plans.
async fn check_weigher_on_arguments_and_demand_control() {
    let router = TestRouter::builder()
        .inline_config(format!(
            r#"supergraph:
  source: file
  path: "{COST_SUPERGRAPH_PATH}"
query_planner:
  allow_expose: true
demand_control:
  enabled: true
  operation_cost:
    max: 1000000000
    mode: measure
  subgraphs_budget:
    mode: measure
cache:
  router:
    parsing:
      max_size: 1GB
  supergraph:
    validation:
      max_size: 1GB
    normalization:
      max_size: 1GB
    query_plans:
      max_size: 1GB
"#
        ))
        .build()
        .start()
        .await;

    let dry_run = e2e::testkit::some_header_map!("hive-expose-query-plan" => "dry-run");

    for index in 0..WARMUP_OPERATIONS {
        let resp = router
            .send_graphql_request(&cost_operation(usize::MAX - index), None, dry_run.clone())
            .await;
        assert!(resp.status().is_success(), "warmup request failed");
    }

    for index in 0..MEASURED_COST_OPERATIONS {
        let resp = router
            .send_graphql_request(&cost_operation(index), None, dry_run.clone())
            .await;
        assert!(
            resp.status().is_success(),
            "cost operation {index} failed to plan"
        );
    }
    quiesce(&router).await;

    let cached_plans = plan_cache_entries(&router);
    assert!(
        cached_plans >= MEASURED_COST_OPERATIONS as u64,
        "expected at least {MEASURED_COST_OPERATIONS} cached plans, found {cached_plans}"
    );

    let before_clear = current_bytes();
    check_weigher_against_reality(&router, "custom-cost", before_clear).await;
}

/// Compares what the weigher charged for the cached entries against the heap that comes
/// back when the caches are dropped. Runs last for a router, since it empties every cache.
async fn check_weigher_against_reality(
    router: &e2e::testkit::TestRouter<e2e::testkit::Started>,
    corpus: &str,
    bytes_before_clear: usize,
) {
    let mut estimated = router.shared_state().parse_cache.weighted_size();
    router.schema_state().for_each_runtime(|runtime| {
        estimated += runtime.validate_cache.weighted_size()
            + runtime.normalize_cache.weighted_size()
            + runtime.plan_cache.weighted_size();
    });
    assert!(
        estimated > 0,
        "the caches are not running with a byte budget, so nothing was weighed"
    );

    router.shared_state().parse_cache.invalidate_all();
    router.schema_state().for_each_runtime(|runtime| {
        runtime.validate_cache.invalidate_all();
        runtime.normalize_cache.invalidate_all();
        runtime.plan_cache.invalidate_all();
    });
    quiesce(router).await;

    let released = bytes_before_clear.saturating_sub(current_bytes());
    let drift = (estimated as f64 - released as f64).abs() / released as f64;

    println!(
        r#"{{"metric":"weigher_accuracy","corpus":"{corpus}","estimated_bytes":{estimated},"released_bytes":{released},"drift":{drift:.3}}}"#
    );

    assert!(
        drift <= ESTIMATE_TOLERANCE,
        "the cache weigher is off by {:.1}% on {corpus}: it charged {estimated} bytes, dropping \
         the caches released {released}. A `HeapSize` impl is most likely missing a field that a \
         cached type grew - see bin/router/src/heap_size.rs.",
        drift * 100.0
    );
}

/// Flushes pending Moka work before measuring
async fn quiesce(router: &e2e::testkit::TestRouter<e2e::testkit::Started>) {
    router.shared_state().parse_cache.run_pending_tasks().await;

    let mut runtimes = Vec::new();
    router.schema_state().for_each_runtime(|runtime| {
        runtimes.push((
            runtime.validate_cache.clone(),
            runtime.normalize_cache.clone(),
            runtime.plan_cache.clone(),
        ));
    });
    for (validate, normalize, plan) in runtimes {
        validate.run_pending_tasks().await;
        normalize.run_pending_tasks().await;
        plan.run_pending_tasks().await;
    }
}

/// Total plan cache entries across all runtimes
fn plan_cache_entries(router: &e2e::testkit::TestRouter<e2e::testkit::Started>) -> u64 {
    let mut total = 0;
    router
        .schema_state()
        .for_each_runtime(|runtime| total += runtime.plan_cache.entry_count());
    total
}
