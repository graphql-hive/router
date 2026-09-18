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

/// Stays below cache capacity to avoid eviction
const MEASURED_OPERATIONS: usize = 800;

/// Warms up one-time allocations before measuring
const WARMUP_OPERATIONS: usize = 25;

/// Retained heap bytes per operation for `grafbase-many-plans`
const BASELINE_BYTES_PER_OPERATION: usize = 5_826;

/// Allowed increase above the baseline
const TOLERANCE: f64 = 0.05;

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

#[ntex::main]
async fn main() {
    let router = TestRouter::builder()
        .inline_config(format!(
            r#"supergraph:
  source: file
  path: "{SUPERGRAPH_PATH}"
query_planner:
  allow_expose: true
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
