// Parse + normalize every corpus operation both ways into the flat AST, from the text with
// `flat::parse` and from the parser's tree like the pipeline, check both give the same
// `NormalizedDocument`, and time them.
//
// MODE=check (default), MODE=bench, MODE=prof (only the flat normalizer, on CASE), or
// MODE=graphs (builds every supergraph's planner graph, which normalizes every field set).
// MANIFEST=<json> and AUDIT_DIR=<dir> add more cases to check.
use std::alloc::{GlobalAlloc, Layout};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use graphql_tools::parser::query::{Definition, OperationDefinition};
use hive_router::query_planner::ast::flat;
use hive_router::query_planner::ast::normalization::normalize_operation;
use hive_router::query_planner::state::supergraph_state::SupergraphState;
use hive_router::query_planner::utils::parsing::{parse_schema, safe_parse_operation};

struct Counting;
static COUNT: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT.load(Relaxed) {
            ALLOCS.fetch_add(1, Relaxed);
            let live = LIVE.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(live, Relaxed);
        }
        unsafe { hive_router::RouterGlobalAllocator.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if COUNT.load(Relaxed) {
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
        unsafe { hive_router::RouterGlobalAllocator.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNT.load(Relaxed) {
            ALLOCS.fetch_add(1, Relaxed);
            let live = LIVE.fetch_add(new_size, Relaxed) + new_size;
            LIVE.fetch_sub(layout.size(), Relaxed);
            PEAK.fetch_max(live, Relaxed);
        }
        unsafe { hive_router::RouterGlobalAllocator.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/");

struct Case {
    label: String,
    sdl_index: usize,
    query: String,
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn operation_names(query: &str) -> Vec<Option<String>> {
    let Ok(document) = safe_parse_operation(query) else {
        return vec![None];
    };
    document
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            Definition::Operation(operation) => Some(match operation {
                OperationDefinition::Query(q) => q.name.clone(),
                OperationDefinition::Mutation(m) => m.name.clone(),
                OperationDefinition::Subscription(s) => s.name.clone(),
                OperationDefinition::SelectionSet(_) => None,
            }),
            _ => None,
        })
        .collect()
}

/// (allocations, peak bytes above the start, bytes still live from it)
fn measure<T>(f: impl FnOnce() -> T) -> (T, usize, usize, usize) {
    let allocs = ALLOCS.load(Relaxed);
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    COUNT.store(true, Relaxed);
    let out = f();
    COUNT.store(false, Relaxed);
    let live = LIVE.load(Relaxed).saturating_sub(base);
    (
        out,
        ALLOCS.load(Relaxed) - allocs,
        PEAK.load(Relaxed) - base,
        live,
    )
}

fn best<T>(runs: usize, mut f: impl FnMut() -> T) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..runs {
        let t = Instant::now();
        let out = f();
        best = best.min(t.elapsed());
        drop(out);
    }
    best
}

fn main() {
    let mode = std::env::var("MODE").unwrap_or_else(|_| "check".into());
    let mut sdls: Vec<String> = Vec::new();
    let mut cases: Vec<Case> = Vec::new();

    let fixtures: &[(&str, &[&str])] = &[
        (
            "../../bench/supergraph.graphql",
            &["../../bench/operation.graphql"],
        ),
        (
            "fixture/abstract-many-subgraphs/supergraph.graphql",
            &["fixture/abstract-many-subgraphs/operation.graphql"],
        ),
        (
            "fixture/bfg/supergraph.graphql",
            &[
                "fixture/bfg/operation-1.graphql",
                "fixture/bfg/operation-2.graphql",
            ],
        ),
        (
            "fixture/grafbase-many-plans/supergraph.graphql",
            &["fixture/grafbase-many-plans/operation.graphql"],
        ),
        (
            "fixture/heavy-query/supergraph.graphql",
            &["fixture/heavy-query/operation.graphql"],
        ),
        (
            "fixture/products-list/supergraph.graphql",
            &["fixture/products-list/operation.graphql"],
        ),
        ("fixture/supergraph.graphql", &["fixture/operation.graphql"]),
        (
            "fixture/wfg/supergraph.graphql",
            &[
                "fixture/wfg/mutation-1.graphql",
                "fixture/wfg/query-1.graphql",
                "fixture/wfg/query-2.graphql",
            ],
        ),
    ];
    for (supergraph, operations) in fixtures {
        sdls.push(read(&format!("{ROOT}{supergraph}")));
        for op in *operations {
            cases.push(Case {
                label: format!("fixture:{op}"),
                sdl_index: sdls.len() - 1,
                query: read(&format!("{ROOT}{op}")),
            });
        }
    }
    if mode == "check" || mode == "graphs" {
        if let Ok(manifest) = std::env::var("MANIFEST") {
            let entries: Vec<serde_json::Value> = serde_json::from_str(&read(&manifest)).unwrap();
            let mut paths: Vec<String> = Vec::new();
            let base = sdls.len();
            for entry in &entries {
                let path = entry["supergraph"].as_str().unwrap().to_string();
                let index = match paths.iter().position(|p| *p == path) {
                    Some(i) => i,
                    None => {
                        paths.push(path.clone());
                        sdls.push(read(&path));
                        paths.len() - 1
                    }
                };
                cases.push(Case {
                    label: format!("manifest:{}", entry["label"].as_str().unwrap()),
                    sdl_index: base + index,
                    query: entry["query"].as_str().unwrap().to_string(),
                });
            }
        }
        if let Ok(audit) = std::env::var("AUDIT_DIR") {
            let mut ids: Vec<_> = std::fs::read_dir(&audit)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            ids.sort();
            for id in ids {
                let tests: serde_json::Value =
                    serde_json::from_str(&read(&format!("{audit}/{id}/tests.json"))).unwrap();
                sdls.push(read(&format!("{audit}/{id}/supergraph.graphql")));
                for (i, t) in tests.as_array().unwrap().iter().enumerate() {
                    cases.push(Case {
                        label: format!("audit:{id}#{i}"),
                        sdl_index: sdls.len() - 1,
                        query: t["query"].as_str().unwrap().to_string(),
                    });
                }
            }
        }
    }

    // one state per supergraph, None when it doesn't build
    let states: Vec<Option<SupergraphState>> = sdls
        .iter()
        .map(|sdl| {
            catch_unwind(AssertUnwindSafe(|| {
                let parsed = parse_schema(sdl);
                SupergraphState::new(&parsed)
            }))
            .ok()
        })
        .collect();

    if mode == "prof" {
        // only the flat normalizer on the cases matching CASE, RUNS times
        let filter = std::env::var("CASE").unwrap_or_else(|_| "wfg/query-1".into());
        let runs: usize = std::env::var("RUNS")
            .ok()
            .and_then(|r| r.parse().ok())
            .unwrap_or(20);
        for case in cases.iter().filter(|c| c.label.contains(&filter)) {
            let state = states[case.sdl_index].as_ref().unwrap();
            let name = operation_names(&case.query).into_iter().next().flatten();
            let doc = flat::parse::parse(&case.query).unwrap();
            let t = Instant::now();
            for _ in 0..runs {
                drop(flat::normalize::normalize_operation(state, &doc, name.as_deref()).unwrap());
            }
            println!(
                "{} {:.1} ms per run",
                case.label,
                t.elapsed().as_secs_f64() * 1e3 / runs as f64
            );
        }
        return;
    }
    if mode == "graphs" {
        // builds the planner's graph for every supergraph, which normalizes every field set
        let (mut built, mut failed) = (0, 0);
        for sdl in &sdls {
            let result = catch_unwind(AssertUnwindSafe(|| {
                hive_router::query_planner::planner::Planner::new_from_supergraph(
                    &parse_schema(sdl),
                    Default::default(),
                )
                .is_ok()
            }));
            match result {
                Ok(_) => built += 1,
                Err(_) => failed += 1,
            }
        }
        println!("{built} graphs built, {failed} panicked");
        return;
    }
    if mode == "check" {
        check(&cases, &states);
    } else {
        bench(&cases, &states);
    }
}

fn check(cases: &[Case], states: &[Option<SupergraphState>]) {
    let (mut same, mut differ, mut skipped) = (0, 0, 0);
    for case in cases {
        let Some(state) = &states[case.sdl_index] else {
            skipped += 1;
            continue;
        };
        let tree = safe_parse_operation(&case.query);
        let flat_doc = flat::parse::parse(&case.query);
        let (tree, flat_doc) = match (tree, flat_doc) {
            (Ok(tree), Ok(flat_doc)) => (tree, flat_doc),
            (Err(_), Err(_)) => {
                same += 1;
                continue;
            }
            (tree, flat_doc) => {
                differ += 1;
                println!(
                    "PARSE DIFFERS {}: tree ok {}, flat {:?}",
                    case.label,
                    tree.is_ok(),
                    flat_doc.err()
                );
                continue;
            }
        };
        for name in operation_names(&case.query) {
            let old = catch_unwind(AssertUnwindSafe(|| {
                normalize_operation(state, &tree, name.as_deref())
                    .map(|d| (format!("{:?}", d.operation), d.operation_name))
                    .map_err(|e| e.to_string())
            }));
            let new = catch_unwind(AssertUnwindSafe(|| {
                flat::normalize::normalize_operation(state, &flat_doc, name.as_deref())
                    .map(|d| (format!("{:?}", d.operation), d.operation_name))
                    .map_err(|e| e.to_string())
            }));
            let (old, new) = match (old, new) {
                (Ok(old), Ok(new)) => (old, new),
                (Err(_), Err(_)) => {
                    same += 1;
                    continue;
                }
                (old, new) => {
                    differ += 1;
                    println!(
                        "PANIC DIFFERS {} {:?}: tree panicked {}, flat panicked {}",
                        case.label,
                        name,
                        old.is_err(),
                        new.is_err()
                    );
                    continue;
                }
            };
            if old == new {
                same += 1;
            } else {
                differ += 1;
                println!("DIFFERS {} {:?}", case.label, name);
                if std::env::var("SHOW").is_ok() {
                    println!("  tree: {:?}\n  flat: {:?}", old, new);
                }
            }
        }
    }
    println!("{same} same, {differ} differ, {skipped} skipped (supergraph didn't build)");
}

/// The pipeline's way (the parser, then the tree turned into the flat form) and the flat
/// parser's.
fn bench(cases: &[Case], states: &[Option<SupergraphState>]) {
    let runs: usize = std::env::var("RUNS")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(20);
    println!(
        "{:<44} {:>10} {:>10} {:>12} {:>12}   {:>9} {:>9}  {:>9} {:>9}  {:>9} {:>9}",
        "case",
        "parse tree",
        "parse flat",
        "norm tree",
        "norm flat",
        "allocs t",
        "allocs f",
        "peak t",
        "peak f",
        "live t",
        "live f"
    );
    for case in cases {
        let Some(state) = &states[case.sdl_index] else {
            continue;
        };
        let name = operation_names(&case.query).into_iter().next().flatten();
        let name = name.as_deref();
        let tree = safe_parse_operation(&case.query).unwrap();
        let flat_doc = flat::parse::parse(&case.query).unwrap();

        let parse_tree = best(runs, || safe_parse_operation(&case.query).unwrap());
        let parse_flat = best(runs, || flat::parse::parse(&case.query).unwrap());
        let norm_runs = runs.clamp(1, 5);
        let norm_tree = best(norm_runs, || {
            normalize_operation(state, &tree, name).unwrap()
        });
        let norm_flat = best(norm_runs, || {
            flat::normalize::normalize_operation(state, &flat_doc, name).unwrap()
        });

        // what's live after is the normalized operation, the parsed document is dropped
        let (kept, allocs_t, peak_t, live_t) = measure(|| {
            let doc = safe_parse_operation(&case.query).unwrap();
            normalize_operation(state, &doc, name).unwrap()
        });
        drop(kept);
        let (kept, allocs_f, peak_f, live_f) = measure(|| {
            let doc = flat::parse::parse(&case.query).unwrap();
            flat::normalize::normalize_operation(state, &doc, name).unwrap()
        });
        drop(kept);
        let label = case
            .label
            .trim_start_matches("fixture:fixture/")
            .trim_start_matches("fixture:");
        let ms = |d: Duration| format!("{:.3}", d.as_secs_f64() * 1e3);
        let mib = |b: usize| format!("{:.1}M", b as f64 / 1048576.0);
        println!(
            "{:<44} {:>10} {:>10} {:>12} {:>12}   {:>9} {:>9}  {:>9} {:>9}  {:>9} {:>9}",
            label,
            ms(parse_tree),
            ms(parse_flat),
            ms(norm_tree),
            ms(norm_flat),
            allocs_t,
            allocs_f,
            mib(peak_t),
            mib(peak_f),
            mib(live_t),
            mib(live_f),
        );
    }
}
