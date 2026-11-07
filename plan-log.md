# plan-log: slot-based response pipeline

Working record for `pipeline-plan.md`. Branch `kamil-pipeline-perf`, cut from
`kamil-perf-runtime-pr7` (`c9bcf575`). Newest entries at the bottom of each section.

## Goals, constraints, success criteria (from the plan)

| # | Criterion | Status |
|---|---|---|
| G1 | Pipeline CPU/request ≥ 2–3x faster (bench op, ct 3 MB, `search`) | **met at the low end** for bench op (2.07x total, ~2.17x pipeline-only) and ct (2.10x), LTO; `search` falls back (unchanged) |
| G2 | ≤ 2 allocations per request in the pipeline, warm (output buffer + errors) | dropped by the user (too strict); reported for information |
| G3 | Peak bytes/request no higher than today | **met**: bench op −56%, ct −29%, search unchanged |
| G4 | Router CPU µs/req and RSS under wrk both improve, neither worse | **met** on the bench op (−22% CPU, −5.5% RSS) and ct unconstrained (−29% CPU, −15% RSS); ct with 1 worker: RSS +4% (see final) |
| G5 | Client output and subgraph request bodies JSON-equal to today | **met** on bench op and ct (bench asserts both), e2e 835/836 with slots forced on, differential 2000/2000, `slots::tests` |
| Gate | End of P3: ≥ 2x on bench op and ct (paired), no more peak memory, ≤ 2 allocs | **met** except the allocation part (G2, dropped) |

Constraints: no unsafe arena with lifetime hazards (index-based storage), `deep_merge` merge
semantics, `project_requires` Missing-vs-Null rules, number grammar validated, depth limit,
unsupported features fall back at plan time (one pipeline per request), old pipeline deleted
after P5.

## Understanding of today's pipeline (read before changing anything)

- Subgraph body → `SubgraphResponse::deserialize_with_shape` (`response/shaped.rs`), a
  shape-aware parser that builds `Value<'a>` (`Vec<(&str, Value)>` objects, borrowed strings,
  numbers parsed to i64/u64/f64). Errors and extensions go through sonic.
  Called from the HTTP executor (`executors/http.rs::deserialize_http_response`).
- Merge: `execution/plan.rs::process_job_result`. Root fetch → `deep_merge` into `ctx.data`.
  Flatten → `take_entities`, `output_rewrites`, `traverse_and_callback_mut` over the flatten
  path, `EntityMerger` (moves on last use, clones otherwise). BatchFetch → per alias.
- Representations: `execution/context.rs::collect_context_entities` walks the flatten path,
  projects each spot with `projection/request.rs::project_requires` into bytes, dedupes by
  `Value::to_hash` (`hash_with_requires`) per context group. Context groups (`@fromContext`)
  and aliased multi-group requests (`AliasedRequests`, `Unalias`) are layered on top.
- Request body: `executors/http.rs::build_request_body` splices raw representation bytes.
- Output: `projection/response.rs::project_by_operation` walks the `ProjectionPlan` over
  `Value`, null bubbling, conditions, type guards, enums, then appends errors (sonic) and
  extensions.
- Teardown: drop of the `Value` tree (`Vec` per object/list).

## Baseline measurements

(pending, P0)

## Log

### 2026-11-06 — start

- Read the plan and the executor (plan.rs, shaped.rs, merge.rs, executors/map.rs, http.rs).
- Workloads: bench op = `bench/operation.graphql` on `bench/supergraph.graphql` (subgraphs
  from `bench/subgraphs`, in-process). ct = `ct-query.graphql` + `ct-super.graphql` +
  `ct-payload.json` (3 MB), which live only in the main checkout
  (`~/Work/guild/hive-router-backup/`), not in git: customer-shaped data, so the bench reads
  them from a path (`CT_DIR`) and skips ct when missing.
  `search`: not defined anywhere in the repo or the slot branches' history. The closest is the
  issue-1309 supergraph (`e2e/src/issues/supergraph.1309.graphql`, `search`/`catalog`/
  `ranking` in `bench/subgraphs`), an interface + `@requires` on a type-conditioned selection.
  Used as the `search` workload; it is tiny (3 listings), so it is a correctness and
  union-width probe, not a throughput number. **Assumption**, flagged.
- `SubgraphExecutorMap` keeps executors in private `DashMap`s; the bench needs a way to put a
  mock executor in. Plan: a small `pub` hook on the map.

### 2026-11-06 — P0 harness and baseline

**Harness**: `bin/router/benches/pipeline_benches.rs` (custom main, `harness = false`).
- Runs the real `execute_query_plan` on a current-thread runtime. Subgraph executors are
  replaced through a new `#[doc(hidden)] SubgraphExecutorMap::replace_http_executor` with a
  `Replay` executor that builds the request body (`build_request_body`, as the HTTP executor
  does) and parses the recorded body with `deserialize_with_shape`, as the HTTP executor does.
- Bodies are recorded at startup from the in-process bench subgraphs (deterministic, no rand).
  Keyed by (subgraph, query) and served in recorded order.
- Before timing, the client output and every request body are checked JSON-equal (key order
  kept, numbers by value) against `benches/fixtures/pipeline/<workload>/{output,requests}.json`
  (ct: `$CT_DIR/pipeline-ref/`). `--bless` writes them. Blessed on `c9bcf575` + harness only.
- Reports thread CPU per request (`CLOCK_THREAD_CPUTIME_ID`, p10/p50/p90), allocations
  (alloc + realloc) and peak bytes over the live heap at request start, from a counting
  wrapper around `RouterGlobalAllocator`. Teardown (dropping data and output) is inside the
  measured region. `NoSubscriber` is set as the global dispatcher.
- Command: `cargo bench -p hive-router --bench pipeline_benches -- [filter] [--iters N]`
  (release profile; see the build note in the final section: no LTO). bench_op output = 87,327 bytes, 7 fetches; search = 3 fetches;
  ct = 1 fetch, 3,000,000-byte output.

**Baseline (`c9bcf575`, release without LTO — see the final build note, two runs)**

| workload | CPU p50 µs | allocs/req | peak KB/req |
|---|---|---|---|
| bench_op | 122.8 / 121.2 | 1082 | 217.1 |
| search | 8.3 / 8.4 | 127 | 11.2 |
| ct | 1654 / 1727 | 13,258 | 4,959.5 |

Allocation counts include executor plumbing (headers, spans, futures, operation names), not
just the pipeline. G2 needs a pipeline-only count; see below.

**In-process profile, bench_op** (samply, `profiling` profile, 30k iterations, inclusive):
parse (`deserialize_with_shape`) 18.9%, representations (`collect_context_entities`) 20.1% of
which `Value::to_hash` 11.9%, merge (`process_job_result`) 13.8% of which
`traverse_and_callback_mut` 12.5% / `deep_merge_from` 7.4%, projection 23.4%,
`build_request_body` 0.3%. Pipeline ≈ 76–78% ≈ 93 µs; the rest (~21%) is executor plumbing
(`SubgraphExecutorMap::execute`, headers, spans, future boxing).

**Whole router** (`profiling` build, `bench/configs/no-deduplication.config.yaml`, bench
subgraphs on :4200, `wrk -t4 -c50`, CPU from `ps` before/after; script
`bench/cpu-per-request.sh <router-bin> <secs> <label>`, bench subgraphs must already run):
- base: 664.6 / 646.6 µs CPU per request, 4.0k / 3.8k req/s, RSS 93 / 90 MB, 0 validation
  failures.
- samply on the router under wrk, weighted by `threadCPUDelta`: **60% of router CPU is in
  `kevent` under mio's `poll`** (idle polling of under-loaded workers, charged as CPU), ~11% in
  read/write syscalls, ~5% hyper client. The pipeline frames: `process_job_result` 4.1%,
  `prepare_job_future` 3.3%, `project_by_operation` 3.0%, parse 2.7%, drop ~1% ≈ **14% of
  router CPU ≈ 90 µs/request**, which matches the in-process number (93 µs): on this laptop
  the caches are not much colder in the router for this op.
- Reading the target as a router number: 2x on the pipeline ≈ −45 µs ≈ −7% router CPU per
  request; 3x ≈ −9%. Excluding the kevent polling (which scales with idleness, not work), the
  pipeline is ~1/3 of the router's real work.

**Skipped in P0, with reasons**
- Baselines on `kamil-slot` / `kamil-slot-projection`: they predate the crate merge
  (`lib/executor`, `lib/query-planner`) and this harness can't run there without a port. Their
  measured numbers are already in the plan; re-measuring them buys no decision. Not done.
- Probe 1 (`@skip`/`@include` bitmask per request): the bench op, ct and search have no
  conditions, so the probe can't move any target workload. Not done.
- Probe 2 (packed flags word from `294683b0`): deferred, the projection walker is 23% of the
  in-process bench; revisit if the new writer is not used.

**Exit P0**: baseline numbers and the pipeline's router share are known. Next: P1.

### 2026-11-06 — design decisions for P1–P3 (before writing code)

Bench op plan shape (from `hive-expose-query-plan: dry-run`): Sequence[ Parallel[Fetch products,
Fetch accounts], Parallel[Flatten(topProducts.@) Fetch inventory (requires), BatchFetch reviews
(2 aliases)], Parallel[BatchFetch products (1 alias, 3 paths), BatchFetch accounts (2 paths)],
BatchFetch inventory (requires price/weight) ]. No rewrites, no context, no conditions.
So the new pipeline must do root Fetch, Flatten+requires and multi-alias multi-path BatchFetch.

**Deviation 1 — layouts per type cluster, not per client selection set.** The plan's
"one superset layout per client selection set" breaks entity sharing: a BatchFetch alias with
several paths parses each entity once, but its paths can sit at client positions with different
selection sets (`product { ...Product reviews {..} }` vs `product { ...Product }` in the bench
op), so the parsed block would need a remap per path, recursively for shared children. Instead:
- union-find over types: every abstract type that shows up as a position (client op, fetch
  selections, `_entities` fragment types) is unioned with its possible types;
- one layout per cluster = every response key selected on any type of the cluster anywhere in
  the client op or any fetch (hidden keys included), `__typename` always;
- every object of a cluster has the same layout, so a parsed entity block copies into any spot
  slot-for-slot and nested objects can be shared by id with no remap.
- Response key → slot per cluster. A key means the same field at every position of the same
  object (GraphQL field merging; the planner aliases conflicts), and different objects at
  different positions never share a block, so keying by response key is safe. Cost: objects
  carry slots for keys their own position doesn't select; measured per workload (open
  question 2).

**Deviation 2 — compiled lazily, once per cached plan.** The slot plan depends on the query plan
and the projection plan. It lives in a `OnceLock` next to the cached `PlannedQuery` and is built
by the first request that uses it; it keeps the `Arc<ProjectionPlan>` it was built from and a
request with a different projection plan (authorization rewrite, introspection partition) uses
the old pipeline.

**Supported set (prototype)**: query operations; Sequence/Parallel/Fetch/Flatten/BatchFetch;
no `context`, no `input_rewrites`, no `output_rewrites`, no context alias templates; no
introspection in the operation; no plugins, no demand control, no subscriptions; projection
plan identical to the compiled one. Anything else → old pipeline for the whole request.

**Semantics kept**: `deep_merge` (Null never overwrites, objects merge slot-wise, lists zip,
else replace), `project_requires` (field looked up by name then alias, keys-known Missing →
`null`, explicit Null kept, nested list items keep their place, `__typename` rules, complete
key check), projection (conditions, guards, enum sets, typename, forced nulls, nullability
shapes, null bubbling, `trim_error_paths`), entity error maps.

**Known difference**: dedupe by hashing written bytes can merge two entities whose bytes are
equal but whose `Value` hashes differ (e.g. a keys-known Missing field vs an explicit null).
Fewer representations in such edge cases; noted if a test hits it.

**Revision of deviation 1 (before coding it)**: layouts per *position*, not per type cluster.
A position is a response path from `data` with list steps and type conditions dropped. Every
fetch shape object maps to a position (root fetch: from `data`; root re-entry: at its merge
path; entity alias: unioned with every merge path of the alias). Unioning positions also unions
their same-key children, recursively (congruence closure). One layout per class = the keys any
fetch writes there. No schema types needed at all, layouts are narrower than type clusters, and
a parsed entity still fits every spot it's copied into. Projection fields, paths and requires
are resolved to slots by walking them along the positions; keys no fetch writes get no slot
(read as Missing).

User note (2026-11-06): `ProjectionPlan` and other executor structures may be changed when it
helps CPU or memory. (`query_planner::ast` stays a public contract.)

### 2026-11-06 — P1–P3 first cut, end to end

Code (all new unless noted):
- `src/executor/slots/compile.rs`: positions → layouts, fetch `Shape`s (re-parses each fetch's
  `document_str` with graphql_tools), path programs, requires programs, projection field slots.
  `SlotPlan::compile` returns `Unsupported(reason)` for anything outside the supported set.
- `src/executor/slots/store.rs`: `Slot` (16 bytes, `Copy`), `Arena` (bodies, slots, items,
  scratch, dedupe maps), thread-local pool (4 arenas, dropped past 1M slots), `merge`/
  `merge_object` with `deep_merge` rules (a Missing target takes even a Null source: `deep_merge`
  adds the key, and `project_requires` tells Null from Missing), `write_whole`.
- `src/executor/slots/parse.rs`: the shaped parser loop writing slots; scalars as byte ranges;
  number grammar + f64 overflow check (only for exponents / >300 digits); string escape check;
  UTF-8 check when non-ASCII; depth limit 250; on failure the caller asks sonic for the message.
- `src/executor/slots/write.rs`: projection over slots, same semantics as
  `projection::response` (made `ShapeCursor` and `trim_error_paths` `pub(crate)`). Errors and
  extensions are serialized straight into the output buffer.
- `src/executor/execution/plan/slots.rs`: plan walk, representations (port of
  `project_requires` over slots), dedupe by object id first, then by hashing the written bytes
  (with a byte compare on a hash hit, so a collision can't merge two entities), entity merge by
  `merge_object` (nested objects shared), batch error split, entity error paths by re-walking
  only when there are errors.
- Wiring: `SubgraphExecutionRequest::raw_response` (HTTP executor returns the body unparsed),
  `PlannedQuery::slots: Arc<SlotPlanCell>` → `PlannedRequest::slot_plan` →
  `QueryPlanExecutionOpts::slot_plan`; `execute_query_plan_with_data` dispatches when
  `slots::usable` (enabled, no plugins, no demand control, no introspection, a query, plan
  compiled with this projection plan). Switch: `slots::set_enabled` or `HIVE_SLOT_PIPELINE=1`.
- Refactor in `plan.rs`: `handle_job_error`, `apply_job_headers` extracted from
  `process_job_result` (no behavior change).

Correctness: bench_op and ct give JSON-equal client output **and** JSON-equal subgraph request
bodies on the first run. search falls back ("rewrites or context on a batch alias").

Pitfall found: the bench's `Replay` executor ignored `raw_response`, so every body was parsed
twice (old parser + slot parser): the slot path looked 5–30% *slower*. Fixed in the mock.

**Paired results** (same binary, `HIVE_SLOT_PIPELINE=0/1`, non-LTO release, 1000 iters, 2 rounds):

| workload | CPU p50 µs old → slots | allocs | peak KB |
|---|---|---|---|
| bench_op | 121.7 / 121.9 → 97.0 / 96.8 (−21%) | 1082 → 777 | 217.1 → 95.3 |
| ct | 1657 / 1674 → 1194 / 1197 (−28%) | 13,258 → 19 | 4,959.5 → 3,516.4 |

Next: profile the slot path, find what's left.

### 2026-11-06 — tuning round 1 (paired, non-LTO, bench_op / ct p50 µs, old path 121.5 / 1665)

| change | bench_op | ct | allocs bench_op | verdict |
|---|---|---|---|---|
| first cut | 97.0 | 1195 | 777 | baseline for tuning |
| no `Box<str>` per object in representations (typename borrowed from the arena, read once per object) | 88.5 | 1225 | 107 | kept |
| fixed 24-byte key windows + 32-byte value windows in the writer (`put_window`, small `unsafe`) | 85.1 | 1181 | 107 | kept |
| single-pass 8-byte SWAR string scan instead of 2 SWAR steps + memchr2 + vectorized check | 90.0 | 1324 | 107 | **reverted**: NEON memchr2 + the auto-vectorized check loop win on long strings |

The 107 allocations left on bench_op are executor plumbing both paths share (header maps,
operation names, spans, boxed futures, request bodies), not pipeline.

Profile of ct on the slot path (LTO): string scanning in the parser ~19% (inherent memchr +
check), writer value copies, `resolve_type_name` ~3.5%: ct's projection has `FieldType`
conditions on interface-typed fields (`BaseMoney`, `AttributeDefinitionType`), each costing two
std `HashMap` (SipHash) lookups in `SchemaMetadata::type_fields` per object. Next: resolve the
static output type per projection field at compile time.

### 2026-11-06 — tuning round 2: representations (bench_op, paired non-LTO, old path ~121.8 µs)

Spot statistics (`SLOTS_DEBUG=1`): bench_op has 335 spots over 6 entity fetches
(5, 5, 6, 130, 59, 130); the 130- and 59-spot fetches collapse to **1** entity each. Every spot
is a distinct object (each review's `product` is its own parsed object), so dedupe by object
id never hits here.

| change | bench_op p50 | verdict |
|---|---|---|
| (before) write every spot's representation, hash the bytes, truncate on a hit | 85.4 | |
| fingerprint each spot from the slots it would read (typename, key presence via `keys.get` + slot search, selected values) with `ahash`, write only new ones | 86.0 | no gain: same cost as writing |
| fingerprint key presence as a bitmask of the object's present slots instead of `keys.get` + key slot search | 85.6 | kept (simpler, no per-spot map lookup) |
| drop the by-object-id map (3 hash-map ops per spot, never hits on bench_op; identical objects get identical fingerprints anyway) | 83.7 | kept |
| replace `ahash::AHasher` in fingerprints with `Mix`: one folded 128-bit multiply per 8 bytes, tags packed into length words, per-process seed | **75.7** | kept |

Profile before the `Mix` change: fingerprinting was 21.6% of the bench (~56 ns/spot), almost
all of it in the dependent chain of `AHasher::update` calls (one per `write_u8` marker).

Soundness of fingerprints: the representation writer is a pure function of what the
fingerprint hashes (the typename, which slots are present, which covers the complete-key check,
and the selected values recursively), so equal fingerprints mean equal representations, up to
64-bit collisions. That's the same trust `Value::to_hash` gets on the old path. New
fingerprints still go through the written-bytes dedupe, which compares bytes on a hash hit.

Where bench_op stands: ~75.7 µs total. Executor plumbing (`prepare_execution_job`: headers,
spans, operation names, the executor map) is ~16 µs on both paths, so pipeline ≈ 60 µs vs
≈ 93 µs before: **~1.55x**. Not at the 2x gate yet.

### 2026-11-06 — P4 correctness: suites with the slot pipeline forced on

- `HIVE_SLOT_PIPELINE=1 cargo test_all`: 2097/2097 pass (few of these reach the slot path).
- `HIVE_SLOT_PIPELINE=1 cargo test_e2e`, first run: 827 passed, 9 failed.
  - 5× `issue_966_custom_scalar_*`: a custom scalar whose JSON has an escaped object key
    (`"entity.root\t"`). The slot parser rejected escaped keys everywhere; the shaped parser
    does too but then lets sonic read the whole body. **Fix**: escaped keys are checked like
    any string and skipped / kept raw (they can never match a shape field).
  - `should_override_subgraph_url_based_on_header_value` and 2× `test_otlp_*transport_failure*`:
    they expect a connection to :4200 to fail, and the bench subgraphs I had left running for
    the wrk measurements answered. Not the slot pipeline: pass once that process is stopped.
    **Lesson**: stop `bench/subgraphs` on :4200 before running e2e.
  - `file_watch_works`: known local failure.
- Second full run with the fix: **835/836 pass** (only `file_watch_works`).
- Coverage of that run (temporary per-request tally, since removed): 859 requests ran on the
  slot path; fallbacks: 443 not a query (subscriptions, mutations), 105 demand control, 42
  introspection, 27 `@fromContext`, 15 plugins, 7 projection plan differs, 6 rewrites,
  5 context alias templates.
- Added the config flag `query_planner.experimental_slot_pipeline` (default false), applied in
  `RouterSharedState::new`; `HIVE_SLOT_PIPELINE=1` still forces it on. The config JSON schema
  still needs regenerating (`cargo router-config`).

### 2026-11-06 — tuning round 3 (paired, non-LTO; old path bench_op ~122.6, ct ~1670)

| change | bench_op | ct | verdict |
|---|---|---|---|
| (start) | 75.7 | 1180 | |
| static output type per projection field, resolved at compile time when the parent is an object type (`SlotPlan::field_types`); the writer skips `SchemaMetadata::get_type_fields` (std `HashMap`, SipHash) for `FieldType` conditions and deferred type names | 75.7 | 1072 | kept (ct has `FieldType` conditions on `BaseMoney`/`AttributeDefinitionType` fields) |
| writer: look the response key up only when the key doesn't fit the window | | | kept |
| parser: the control/non-ASCII check after `memchr2` as a `min`/`max` reduction (vectorized) instead of `|=` on bools (scalar) | 73.0 | 1021 | kept |

ct value stats: 18.5k strings = 2.6 MB of the 3 MB body, 8.5k of them longer than 64 bytes, so
parse on ct is mostly string scanning (memchr2 + the check pass).

### 2026-11-06 — G2: where the slot path's allocations are (bench_op, `--trace-allocs`)

107 per request. About 55 are executor plumbing shared with the old path (FuturesUnordered
tasks and queues, boxed job futures, `prepare_execution_job` internals, executor map futures).
About 50 are slot-path bookkeeping `Vec`s: per-path target lists (17), merge paths (5),
per-alias error buckets (5), alias lists (4), merge records (4), plus the representation
buffers and variable lists moved into each subgraph request (they go into
`raw_variable_values: Vec<(&str, Vec<u8>)>`, which owns them). Bookkeeping can be pooled in the
arena. The representation buffers need `raw_variable_values` to borrow (an executor API change),
so **G2 (≤ 2) is not met as stated**. ct (one root fetch): 19 allocations total.

### 2026-11-06 — G4: whole router under wrk

Same binary (`profiling` profile, LTO), `HIVE_SLOT_PIPELINE=0/1`, no-dedupe config, bench
subgraphs, `wrk -t4 -c50 -d20s`, 3 paired rounds, 0 validation failures
(`bench/cpu-per-request.sh`):

| | CPU µs/req | req/s | RSS KB | p99 (info) |
|---|---|---|---|---|
| slots=0 | 626.5 / 632.1 / 638.8 | 4727 / 4870 / 4837 | 92656 / 92304 / 92112 | 27.9 / 26.1 / 25.5 ms |
| slots=1 | 510.6 / 509.2 / 530.4 | 5045 / 5207 / 5008 | 87712 / 87936 / 88416 | 22.1 / 21.0 / 27.1 ms |

**−18% router CPU per request (−115 µs), −4.8% RSS.** Larger than the in-process saving
(~−50 µs), as the plan expected from `kamil-slot` (colder caches and allocator pressure in the
router make the old pipeline cost more there). G4 met on this laptop.

### 2026-11-06 — P4 differential

Two routers from the same `profiling` binary on the bench supergraph (`bench/configs/default.config.yaml`,
dedupe on), `PORT=4001 HIVE_SLOT_PIPELINE=0` (baseline) and `PORT=4002 HIVE_SLOT_PIPELINE=1`
(candidate), bench subgraphs on :4200:
`GRAPHQL_DIFF_QUERIES=2000 target/debug/graphql-diff http://localhost:4001/graphql http://localhost:4002/graphql bench/schema.graphql`
(run from a scratch dir, it writes `failed-tests/`). **2000/2000 match** (the tool compares
`data` only).

Where the gate stands (in-process, non-LTO, paired): bench_op 122.6 → 73.0 µs total; minus
~16 µs of executor plumbing on both sides, pipeline ≈ 93 → 57 µs ≈ **1.63x**. ct 1670 →
1021 µs ≈ **1.64x**. Peak memory −56% / −30%. Gate (2x) not reached yet.

**User decision (2026-11-06): G2 (≤ 2 allocations per request) is too strict, ignore it.**
Allocation counts are still reported for information.

### 2026-11-06 — tuning round 4: fingerprint micro-fixes, verified raw passthrough (deviation 3)

Fingerprint: hash the `__typename` slot's raw bytes (different bytes only make fingerprints
differ, which is safe), decode it only when an inline fragment needs it, read tails with
overlapping loads instead of a `memcpy`. bench_op 73.0 → 72.0 µs.

**Deviation 3 — verified raw passthrough subtrees** (not in the plan). ct's writer walked ~34k
values one by one (~11 ns each). At compile time a projection field with children qualifies when:
- one shape object (of one fetch) writes its position and every position below it,
- exactly one projection field reads that position (two type variants reading one position, like
  `meta { a }` for a Widget and `meta { a b }` for a Gadget, rule it out),
- no path walks through it and no `@requires` reads it (`touched` set),
- the client fields are exactly the fetch's fields there, in order, recursively, none of them
  `__typename`, forced null, guarded, or with a condition other than an enum check or a type
  condition its static type always meets (a passthrough object has no `__typename`, so its
  runtime type is the static one).
The parser then checks such a value as it reads it (`Parser::check`): exactly those keys in
order, no null where the nullability shape says non-null (lists included), enum strings in the
allowed set. If it holds, the value is one raw range and the writer copies it in one go; if not,
it rewinds and reads it the normal way, so null bubbling, enum errors and missing keys behave as
before.

| change | bench_op | ct |
|---|---|---|
| before | 72.0 | 1021 |
| passthrough, no conditions allowed | 71.4 | 841 |
| + enum checks and statically true type conditions | 71.5 | 806 |

ct paired: 1666 → 806 µs = **2.07x** (non-LTO).

Correctness:
- New `src/executor/slots/tests.rs`: one-subgraph supergraph with enums and non-null lists,
  17 bodies (key order, missing/extra keys, nulls in non-null positions, invalid and escaped
  enums, escapes, whitespace, number forms, wrong kinds, errors, invalid JSON) through both
  pipelines, JSON-equal, and asserting the passthroughs are really compiled in.
- e2e with slots: first run after passthrough failed
  `nested_abstract_overlap_reresolves_merged_children` (the Widget/Gadget case above). Fixed with
  the single-reader rule, regression test `a_position_two_fields_read_is_never_passed_through`.
  Rerun: 835/836 (only `file_watch_works`).

### 2026-11-06 — tuning round 5: bench_op to the gate

Profiling correction: every profile so far included the bench's one-off recording run, where
`reqwest::Client::new()` (loading native certificates) was ~12% of all samples, counted as
"executor plumbing". Restricted to the timed loop (`pipeline_benches::measure`), plumbing is
~4–7% on both paths (~5 µs), so the old bench_op pipeline is ≈ 117 µs, not 93.

| change | bench_op p50 | allocs |
|---|---|---|
| (start) | 72.2 | 107 |
| fingerprint over `Requires::reads`: every field the selection may read per level, fragments flattened at compile time; no fragment checks, type lookups or typename decoding per spot (the raw `__typename` bytes, hashed anyway, decide which fragments apply) | 65.2 | 107 |
| alias targets in one pooled `Vec` per alias (`Targets`), `SmallVec` for paths/merges/error buckets; removed a leftover `std::env::var("SLOTS_DEBUG")` per `collect` | 64.8 | 70 |
| path walk level by level over a frontier instead of recursively | **63.4** | 70 |

Paired (same binary): old 123.6 µs → 63.4 µs total (1.95x); minus ~5.3 µs plumbing on both
sides, **pipeline 118 → 58 µs ≈ 2.04x**. ct 2.07x (round 4). Both gate workloads clear 2x on
non-LTO builds; LTO confirmation below.


### 2026-11-06 — final measurements

**Build note.** The root `Cargo.toml` `[profile.release]` has `opt-level`/`lto`/
`codegen-units` commented out ("for faster compilation"), edited at 15:24, not by this work and
left as is. So the P0 baseline and every router binary here were release builds without LTO
(codegen-units 16). Every comparison is paired on one binary (`HIVE_SLOT_PIPELINE=0/1`), so the
ratios hold. The in-process table below forces LTO (`CARGO_PROFILE_RELEASE_LTO=true`,
`CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1`).

**In process (pipeline bench), LTO, 3 paired rounds × 2000 iterations, p50 µs:**

| workload | old | slots | speedup | allocs old → slots | peak KB old → slots |
|---|---|---|---|---|---|
| bench_op | 126.4 / 126.5 / 126.2 | 60.5 / 61.3 / 61.0 | **2.07x** total; ≈ 2.17x without the ~5 µs of shared executor plumbing | 1082 → 70 | 217.1 → 95.3 |
| ct (3 MB) | 1618 / 1622 / 1617 | 775 / 772 / 770 | **2.10x** | 13,258 → 18 | 4,959.5 → 3,516.4 |
| search | 8.1 / 8.2 / 8.0 | 8.2 / 8.1 / 8.0 | 1.0x: falls back ("rewrites or context on a batch alias") | 127 → 127 | 11.2 → 11.2 |

**Whole router, bench op** (no-dedupe config, bench subgraphs, `wrk -t4 -c50 -d20s`, 3 paired
rounds, 0 validation failures, `bench/cpu-per-request.sh`):

| workers | | CPU µs/req | req/s | RSS KB | p99 |
|---|---|---|---|---|---|
| default (10) | old | 582 / 606 / 600 | 4860 / 4964 / 4995 | 94384 / 92208 / 90704 | 27.9 / 23.8 / 22.9 ms |
| default (10) | slots | 455 / 471 / 465 (**−22%**) | 5122 / 5192 / 5205 (+4.7%) | 87392 / 87456 / 87200 (**−5.5%**) | 23.5 / 20.6 / 19.6 ms |
| 2 | old | 366 / 370 / 367 | 4585 / 4589 / 4640 | ~80 MB | ~16.0 ms |
| 2 | slots | 301 / 300 / 303 (−18%) | 4977 / 5006 / 4984 (+8.3%) | ~82 MB (noisy) | ~14.8 ms |
| 1 | old | 326 / 325 / 326 | 3044 / 3070 / 3061 | 84544 / 79424 / 78336 | 19.7 / 18.9 / 19.0 ms |
| 1 | slots | 248 / 249 / 248 (**−24%**) | 4000 / 3983 / 4004 (**+30.6%**) | 78336 / 85392 / 78512 | 15.3 / 15.9 / 15.5 ms |

Why req/s moves little unless the router is the bottleneck: on this laptop the bench subgraphs
(7 subgraph requests per router request, one of them 79 KB) use ~4 cores at 3k req/s and ~5.3
at 4k, and wrk takes more; with all router workers the router used ~2.3 of 10 cores. With one
worker the router is the bottleneck, and req/s follows its CPU per request (+30.6%).

**Whole router, ct** (ct supergraph, `serve-payload.mjs` on :4200 for every subgraph,
`wrk -t2 -c8`, 15 s, 3 paired rounds, 3,000,000-byte responses; `<scratchpad>/steps/cpu-ct.sh`):

| workers | | CPU µs/req | req/s | RSS KB |
|---|---|---|---|---|
| default | old | 3278 / 3353 / 3331 | 1157 / 1168 / 1179 | 186176 / 168560 / 163840 |
| default | slots | 2351 / 2340 / 2366 (**−29%**) | 1292 / 1295 / 1247 (+9%) | 148176 / 145600 / 147888 (**−15%**) |
| 1 | old | 2599 / 2599 / 2598 | 385 / 385 / 385 | 109712 / 112400 / 110160 |
| 1 | slots | 1645 / 1633 / 1643 (**−37%**) | 608 / 613 / 608 (**+58%**) | 116032 / 113840 / 115744 (+4%) |

The +4% RSS with one worker is the one place memory went up: 58% more requests in flight per
second through one worker; peak bytes per request are −29% in process.

**Tests:** `cargo test_all` 2100/2100 (default), slots unit tests 3/3, `cargo test_e2e`
835/836 both with the slot pipeline forced on and off (only the known `file_watch_works`),
differential 2000/2000, `cargo clippy` clean for the new code (pre-existing warnings elsewhere),
`cargo fmt --check` clean.

## Where things stand / what's left

Done: P0 (harness, baselines, router share), P1–P3 (compiler, storage, parser, writer,
entity calls, all behind `query_planner.experimental_slot_pipeline` / `HIVE_SLOT_PIPELINE=1`),
P4 (suites forced on, differential, wrk). Not done:
- **P5**: `@fromContext`, rewrites, context alias templates, demand control, plugins on
  execution, subscriptions, mutations, introspection still use the `Value` pipeline (search
  falls back for this reason). The old pipeline can't be deleted until these are ported, and
  until then fixes land twice.
- G2 (≤ 2 allocs) dropped by the user. 70 allocations remain on bench_op, ~55 of them executor
  plumbing shared by both paths; the rest are representation buffers and variable lists that
  `raw_variable_values` owns.
- Whole-router samply re-profile after the change (P4 bullet) not redone; the wrk CPU numbers
  stand in.
- Open question 1 (invalid-JSON messages): kept sonic's wording by asking sonic for the error
  when the slot parser rejects a body. Open question 2 (superset width): position layouts are
  narrower than type clusters; not separately measured on a wide-union workload, since no such
  workload exists in the repo (the `search` workload falls back).
- Kept from `kamil-slot`: fixed-window key/value writes and raw scalars. Not ported: the
  arena tree. New beyond the plan: position layouts (deviation 1), lazily compiled slot plans
  (2), verified raw passthrough subtrees (3), fingerprint dedupe over flattened reads.

Files: `bin/router/src/executor/slots/{mod,compile,store,parse,write,tests}.rs`,
`bin/router/src/executor/execution/plan/slots.rs`, small changes in `plan.rs`,
`executors/{common,http,map,graphql_transport_ws}.rs`, `projection/{plan,response}.rs`,
`pipeline/{mod,execution,query_plan}.rs`, `config/query_planner.rs`, `shared_state.rs`; bench
`bin/router/benches/pipeline_benches.rs` + `benches/fixtures/pipeline/`;
`bench/cpu-per-request.sh`. Nothing committed.

### 2026-11-06 — `per_worker` client mode, and against `origin/main`

Config for all runs below: no-dedupe + `traffic_shaping.http_client_mode: per_worker`
(`bench/configs/no-deduplication.config.yaml` plus that line), optionally `http.workers: 1`.

**Same branch binary, slots off vs on** (non-LTO build, 3 paired rounds):

| workload | workers | CPU µs/req off → on | req/s off → on | RSS off → on |
|---|---|---|---|---|
| bench op | all | 539 → 407 (−24.5%) | 4790 → 4993 (+4.2%) | ~88.2 → ~88.6 MB |
| bench op | 1 | 328 → 251 (−23.3%) | 3038 → 3935 (+29.5%) | ~79.6 → ~78.6 MB |
| ct | all | 3251 → 2181 (−33%) | 1195 → 1286 (+7.6%) | ~166 → ~146 MB (−12%) |
| ct | 1 | 2634 → 1665 (−37%) | 380 → 601 (+58%) | ~108 → ~115 MB (+6%) |

**`origin/main` (`ea661773`) vs this branch** (`447728d3` = main + 3 shaped-parser commits from
`kamil-perf-runtime-pr7` + the slot pipeline). Both built with LTO forced the same way
(`CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1`, since the branch's
`Cargo.toml` has the release profile commented out and main's doesn't); main from a temporary
detached worktree. 3 alternating rounds, 0 validation failures:

| workload | workers | origin/main | branch, slots off | branch, slots on | slots on vs main |
|---|---|---|---|---|---|
| bench op CPU µs/req | all | 549 / 549 / 554 | 518 / 520 / 523 | 389 / 390 / 387 | **−29%** |
| bench op req/s | all | 4900 / 4980 / 4983 | 5059 / 5077 / 5040 | 5194 / 5262 / 5334 | +6% |
| bench op RSS | all | ~90.2 MB | ~84.6 MB | ~84.9 MB | −6% |
| bench op CPU µs/req | 1 | 347 / 361 / 349 | 322 / 328 / 322 | 241 / 244 / 241 | **−31%** |
| bench op req/s | 1 | 2874 / 2665 / 2856 | 3092 / 3009 / 3097 | 4094 / 4026 / 4107 | **+43–46%** |
| ct CPU µs/req | all | 4252 / 4232 / 4276 | 3126 / 3153 / 3151 | 2135 / 2142 / 2137 | **−50%** |
| ct req/s | all | 1161 / 1164 / 1162 | 1207 / 1199 / 1197 | 1290 / 1290 / 1285 | +11% |
| ct RSS | all | ~163.6 MB | ~155.3 MB | ~143.0 MB | −13% |
| ct CPU µs/req | 1 | 3495 / 3473 / 3484 | 2550 / 2546 / 2536 | 1631 / 1631 / 1646 | **−53%** |
| ct req/s | 1 | 287 / 288 / 286 | 393 / 393 / 395 | 612 / 613 / 607 | **2.13x** |

**Why req/s barely moves with all workers (user question: "is there a mutex/lock/wait?").**
Measured, bench op, all workers (`<scratchpad>/steps/who.sh`):

| | connections | req/s | p50 | router cores | subgraph cores |
|---|---|---|---|---|---|
| origin/main | 50 | 4790 | 9.3 ms | 2.62 | 5.86 |
| slots | 50 | 5140 | 8.8 ms | 1.98 | 6.33 |
| origin/main | 200 | 4791 | 40.7 ms | 3.17 | 5.26 |
| slots | 200 | 5125 | 38.1 ms | 2.42 | 5.79 |

`top` during a run: **0.0% idle**, subgraphs ~5.3–6.5 cores, router ~1.6–2.0, wrk ~0.5 on
the 10-core laptop (8 P + 2 E). 4x the connections gives the same req/s and 4x the latency:
the machine is saturated, mostly by the bench subgraphs. A router-side lock would show as idle
CPU with growing latency; there is none. CPU the router stops using goes to the subgraphs
(5.86 → 6.33 cores), so all-workers req/s moves only ~6–11%. With one router worker the
router is the bottleneck and req/s follows its CPU per request (+43% bench op, 2.13x ct).
Showing the all-workers gain needs the subgraphs and load generator on other machines.

### 2026-11-06 — worker count sweep (origin/main vs branch with slots)

Same LTO binaries and no-dedupe + `per_worker` config as above, `http.workers` 2 / 4 / 8,
3 alternating rounds, 0 validation failures (`<scratchpad>/steps/workers.sh`). Averages:

| workload | workers | CPU µs/req main → slots | req/s main → slots | RSS main → slots | router cores used (req/s × CPU/req) main / slots |
|---|---|---|---|---|---|
| bench op | 1 | 352 → 242 (−31%) | 2798 → 4076 (**+46%**) | ~74.5 → ~75.4 MB | 0.98 / 0.99 |
| bench op | 2 | 369 → 283 (−23%) | 4763 → 5271 (+10.7%) | ~78.0 → ~79.6 MB | 1.76 / 1.49 |
| bench op | 4 | 488 → 379 (−22%) | 5092 → 5505 (+8.1%) | ~76.5 → ~82.7 MB | 2.48 / 2.09 |
| bench op | 8 | 552 → 389 (−29.5%) | 5145 → 5410 (+5.2%) | ~87.7 → ~86.7 MB | 2.84 / 2.10 |
| ct | 1 | 3484 → 1636 (−53%) | 287 → 611 (**2.13x**) | ~111.5 → ~110.5 MB | 1.00 / 1.00 |
| ct | 2 | 3453 → 1689 (−51%) | 578 → 1180 (**2.04x**) | ~136 → ~122 MB | 2.00 / 1.99 |
| ct | 4 | 3762 → 2018 (−46%) | 1001 → 1339 (+34%) | ~140 → ~129 MB | 3.77 / 2.70 |
| ct | 8 | 4239 → 2106 (−50%) | 1156 → 1298 (+12%) | ~154 → ~137 MB | 4.90 / 2.73 |

p99, bench op: 2 workers 15.9 → 18.9 ms, 4 workers 32.1 → 18.3 ms, 8 workers 23.1 → 19.1 ms.

Reading it: req/s follows the CPU saving exactly while the router is the bottleneck (its used
cores ≈ its worker count: bench op at 1 worker, ct at 1–2 workers, main's ct at 4). Past that
the cap is outside the router: the bench op tops out at ~5.3–5.5k req/s with the laptop at 0%
idle (bench subgraphs ~6 cores), ct with slots tops out at ~1.3k req/s for 4 and 8 workers
while the router uses only ~2.7 cores, which points at the single-threaded Node payload server
(~3.9 GB/s out). CPU per request also grows with the worker count on both builds (bench op main
352 → 552 µs, slots 242 → 389 µs): idle workers polling `kevent`, seen in the first router
profile.

### 2026-11-06 — ChilliCream graphql-federation-benchmarks: Hive vs Fusion

Their repo (`~/Work/chillicream/graphql-federation-benchmarks` @ c7c822c, apollo-federation
family, HotChocolate 16.7.0 subgraphs on .NET 10), on this laptop (M1 Max, 10 cores, no CPU
pinning: gateway, subgraphs and k6 share the cores). 1 warmup + 3 measured runs, 30 s
cooldown (no temperature sensor on macOS). Gateways:
- hive-main: `origin/main` LTO binary; hive-slots: branch LTO binary with `HIVE_SLOT_PIPELINE=1`.
  Both use the repo's config plus `per_worker`, subgraph dedupe off,
  `max_connections_per_host: 256`.
- fusion-net-11 / fusion-aot-11: HotChocolate Fusion 16.7.3 (latest stable), .NET 11 preview
  JIT with runtime-async / Native AOT, the repo's `fusion-fed` gateway.

Medians of 3 runs (CPU from `ps %cpu`, approximate on macOS):

| scenario | gateway | req/s | p95 ms | peak RSS MB | gateway cores | CPU µs/req |
|---|---|---|---|---|---|---|
| synthetic (`constant`) | hive-main | 3267 | 23.0 | 126 | 2.47 | 757 |
| | hive-slots | 3464 | 21.3 | 132 | 2.11 | 609 |
| | fusion-net-11 | 3899 | 18.5 | 328 | 3.11 | 798 |
| | fusion-aot-11 | 3834 | 18.4 | 126 | 3.16 | 821 |
| burst | hive-main | 3322 | 160.6 | 462 | 2.34 | 706 |
| | hive-slots | 3563 | 153.1 | 458 | 1.96 | 548 |
| | fusion-net-11 | 3844 | 121.3 | 321 | 3.01 | 784 |
| | fusion-aot-11 | 3839 | 120.6 | 173 | 3.11 | 810 |
| real-world (`constant-latency`, 4 ms per subgraph call) | hive-main | 1653 | 37.9 | 118 | 1.44 | 871 |
| | hive-slots | 1667 | 37.9 | 120 | 1.22 | 734 |
| | fusion-net-11 | 1931 | 32.2 | 294 | 1.60 | 829 |
| | fusion-aot-11 | 1924 | 32.3 | 128 | 1.68 | 871 |

0 failed requests anywhere; no leftover monitor/watch processes.

Why Fusion is ahead (measured with a counting proxy in front of the subgraphs, one request):
Hive makes 7 subgraph calls in 4 sequential steps (products+accounts → inventory+reviews →
products+accounts → inventory, the last one because inventory `@requires` price and weight),
Fusion 4 calls in 2 steps. Fusion's entity reuse skips the last two steps (accounts `name`,
products `name price weight`, inventory `inStock shippingEstimate` of the nested entities,
which it already has from the first step).
Same client response (87,327 B, JSON-equal). Real-world: 50 VUs / req/s gives ~30 ms per
request for Hive vs ~26 ms for Fusion (two extra 4 ms round trips on Hive's critical path). Synthetic and burst: the laptop
is shared and saturated, and 7 vs 4 HotChocolate calls per request cost subgraph CPU.

The query, the four subgraphs and the 87,327 B response are the same as our `bench_op`
(`bench/operation.graphql`), so perf work iterates on our own bench.

### 2026-11-06 — round 2: beat Fusion without entity reuse? (non-LTO release from here on)

All on our own bench (`bench_op` is the ChilliCream query, byte-identical response), non-LTO
release builds, no-dedupe + `per_worker`, 1 worker unless noted.

Whole-router samply profile (bench_op, slots, 1 worker, ~248 µs/req): syscalls 32%
(`writev` 17%, `recv` 15%; about 3/4 of it on the hyper client to the subgraphs), hyper
client machinery ~15%, ntex ~10%, the slot pipeline ~27% (parse 10%, collect 7.4%, write 6.6%,
merge 1.8%), allocator ~5%, request handling outside execution ~4.5%. No single hotspot: the
cost is per HTTP exchange (8 per request).

Experiments (3 alternating rounds, CPU µs/req):
- hyper `http1_writev(false)`: 249–251 vs 247–249 base. No change. Reverted.
- hyper `http1_read_buf_exact_size(64 KiB)`: 250–252, more RSS. Reverted.
- Memory at 500 connections (wrk -c500, all workers): peak RSS 282–288 MB (`per_worker`),
  235 MB (`shared`), 80 MB at 50 connections. dhat at 500 concurrent: t-gmax 179 MB, of which
  hyper's read buffers 58 MB (a body read in one chunk keeps the connection's whole adaptive
  buffer alive for the request), our `read_body` copies 24 MB, response buffers ~36 MB.
- `http1_read_buf_exact_size(16 KiB)`: peak RSS at c=500 −17% (282–288 → 234–237 MB), CPU on
  bench_op unchanged (249 vs 248), but ct (3 MB) +14% CPU / −20% req/s (1625 → 1847 µs).
  8 KiB: −24% RSS, +2% CPU on bench_op. 32 KiB: no RSS gain. `http1_max_buf_size(128 KiB)`:
  no RSS gain. Rejected: the ct regression.

Conclusion: without entity reuse the remaining router costs are per subgraph exchange, and the
gap to Fusion is the 3 extra exchanges (7 vs 4) and 2 extra sequential steps. Moving to entity
reuse, opt-in.

### 2026-11-06 — entity reuse on the slot pipeline (opt-in)

User rule: try without entity reuse first; if that makes no progress, implement it behind an
opt-in flag. It didn't (round 2 above), so:

- Planner part ported from the earlier `Value`-pipeline work on `kamil-perf-runtime-pr5/pr6`
  (a882fa65, 9f62826b, 1b866e5a, 77fedafc): every eligible entity fetch (query, one entity type,
  plain output fields, no context/rewrites) gets `EntityReuse { sources, copy }`, the positions
  earlier waves filled with that type and those fields. Mutations, aliased keys and aliases
  sharing a representations variable are left out. `#[serde(skip)]`, plans serialize as before.
- Config ported from 1fc64b3c: `traffic_shaping.experimental_entity_reuse` (default false),
  env `ROUTER_EXPERIMENTAL_ENTITY_REUSE=true`. Process-wide like the slot pipeline switch
  (`slots::set_entity_reuse`), only the slot pipeline copies.
- Slot compile: with the flag on, an alias whose copy selection is leaf fields gets a
  `ReuseProgram`: the target layout, the copied fields' slots, and per source a path program,
  the alias's representation reads resolved at the source position, and the source's slots of
  the copied fields. With the flag off nothing is compiled, so plans are exactly as before.
- Slot runtime (`collect`): sources are walked and every object whose top-level representation
  fields are all present and non-null and that has every copied field is indexed by its
  fingerprint (no key mask: the mask depends on the layout). A spot of the target layout whose
  fingerprint matches gets the fields copied slot to slot and no entity; when no spot is left
  the fetch isn't sent. Nothing is copied once the request has an error.

Checks:
- `pipeline_benches` (`PIPELINE_ENTITY_REUSE=1`): bench_op output JSON-equal to the same
  reference with 4 subgraph requests instead of 7 (`requests-reuse.json`); search unchanged.
  In process: 62.5 → 57.6 µs/req.
- e2e with slots and reuse forced on (`HIVE_SLOT_PIPELINE=1 ROUTER_EXPERIMENTAL_ENTITY_REUSE=true`):
  836/839, the 2 extra failures are the entity_batching call-count snapshots (7 → 4 calls, data
  equal) plus the known file_watch_works. Default e2e 838/839 (file_watch_works), test_all
  2105/2105, new e2e `entity_reuse` (3 tests, from the old branch, on the slot pipeline),
  planner tests (5).
- graphql-diff vs the old pipeline without reuse: 2000/2000 (default config, dedupe on) and
  3000/3000 (no dedupe), data only.

Router, bench_op, non-LTO release, 3 alternating rounds, CPU µs/req and req/s:

| setup | reuse off | reuse on |
|---|---|---|
| 1 worker | 249.4 µs, 3988 req/s | 187.5 µs (−25%), 5221 req/s (+31%) |
| all workers | ~404 µs, ~5388 req/s | ~278 µs (−31%), ~6385 req/s (+18%) |
| all workers, subgraphs delayed 4 ms | ~457 µs, ~1715 req/s, p99 ~33 ms | ~323 µs, ~2917 req/s (+70%), p99 ~21 ms |

Flag off vs the build before the change (`r-base`): 249.4 vs 249.5 µs, no regression.

### 2026-11-07 — Hive (slots + reuse) vs Fusion, own harness

ChilliCream's k6 runner is too slow to iterate with, so `<scratchpad>/cc/quick.sh`: the same
HotChocolate subgraphs (`BENCHMARK_SUBGRAPH_DELAY_MS` for the real-world delay), the same Fusion
16.7.3 builds, Hive non-LTO release with the benchmark's config (`max_connections_per_host: 256`,
`per_worker`, dedupe off), `wrk` + `bench/wrk.lua` (every response validated against the
identical expected JSON), a discarded 20 s warm-up through one gateway, then 20 s runs,
gateways alternating, 3 rounds. Medians:

| scenario | Hive slots | Hive slots + reuse | Fusion AOT |
|---|---|---|---|
| synthetic, c=50 | 4144 req/s, p99 20.6 ms, ~619 µs | **5611**, p99 16.0, ~452 µs | 4716, p99 15.8, ~808 µs |
| real-world (4 ms), c=50 | 1783, p99 35.0 | **2900**, p99 22.5 | 1984, p99 29.2 |
| c=500 | 4081, p99 211, 952 MB | **5546**, p99 156, 748 MB | 4812, p99 129, 258 MB |

Fusion JIT only got ~2.6k req/s here (3.9k in their runner): its JIT needs a longer warm-up than
5 s, so it's left out; AOT is the reference.

Memory at c=500 follows the subgraph connection count: Hive ~1.7–3.7k connections
(`per_worker`, 256 per host), `shared` ~470–510, Fusion 84; ~0.3 MB per connection. dhat on
this setup: hyper read buffers and per-request response buffers dominate, 113 MB still live
after the load. Read buffer knobs don't fix it (exact 16/32 KiB, max 64 KiB: same MB per
connection); capping idle connections per pool makes it churn (26: ~2k req/s). What works is
fewer requests in flight to the subgraphs: `max_connections_per_host` 100 → 489–512 MB, p99
128–129 ms; 64 → 424–428 MB, p99 118–124 ms (better than Fusion AOT), same req/s, no cost at
c=50. The rest of the gap is hyper's legacy pool racing a new connect against a returning
connection and keeping both.

## 2026-11-07 — Strict HTTP/1 pool (why Hive opens so many subgraph connections)

**Why so many connections.** Probed sockets every 2s at c=500 (ChilliCream subgraphs, reuse on). No churn: distinct sockets seen == peak, so it's all pool growth.
- hyper's legacy pool: a checkout that finds no idle connection starts a connect and keeps it even if an idle one comes back first. A finished connection only returns to the pool from a task spawned after the response, so checkouts miss constantly. With `max_connections_per_host: 64` (≤256 in flight) it still opened 1,646 sockets.
- `per_worker` multiplies that by 10 pools: 2,924 sockets (per_worker) vs 1,221 (shared) vs Fusion 256 (one .NET pool).
- Each idle socket costs ~184KB RSS, mostly hyper's adaptive read buffer (grows to 128–256KB on big responses, never shrinks).

**What was tried** (c=500, mcph 100, rps / router CPU µs/req / peak RSS):
| variant | CPU | RSS | note |
|---|---|---|---|
| legacy hyper pool | ~410 | ~490MB | 1,100+ sockets |
| strict per-worker pool | ~398 | ~350MB | ~900 sockets |
| + origin-wide cap = mcph | ~520 | ~235MB | +25% CPU: at saturation idle conns are on other workers, borrowing wakes two threads per exchange |
| cap + return to home worker | ~520 | ~235MB | barely helps, borrows are constant |
| strict pool + 64KB max read buffer | ~405 | ~287MB | **kept** |
| strict pool + 32KB | ~395 | ~272MB | rps −2% |
The 3MB `ct` workload: 2,640 → 2,630 µs/req, no cost from the 64KB buffer.

**Kept:** `executors/h1_pool.rs`, per-worker strict pool for plain `http://`, opt-in via `HIVE_EXP_POOL=1` on `per_worker` mode (https and `allow_only_http2` stay on hyper's client).

**Final** (3 rounds, medians):
| | legacy | pool | Fusion AOT |
|---|---|---|---|
| c=500 rps | 5,535 | 5,712 | 4,759 |
| c=500 p99 | 131ms | 128ms | 131ms |
| c=500 CPU µs/req | 416 | 407 | 780 |
| c=500 peak RSS | 524MB | 287MB | 233MB |
| c=50 rps | 5,614 | 5,703 | 4,680 |
| c=50 CPU µs/req | 455 | 442 | 808 |
| c=50 peak RSS | 128MB | 102MB | 120MB |

Checks: e2e with the pool forced everywhere 833/839 → 838/839 after matching the connect error wording (only `file_watch_works` left); `test_all` 2105/2105; clippy clean on touched files.
Known limits: idle connections expire only when the worker uses that origin again; a response head over 64KB would fail (hyper's default allows ~400KB).

### 2026-11-07 — the pool as config: `traffic_shaping.experimental_http1_pool`
`HIVE_EXP_POOL` replaced by `traffic_shaping.experimental_http1_pool` / `ROUTER_EXPERIMENTAL_HTTP1_POOL`. The pools sit next to either client mode now, so shared mode (the default) can use them too.

ChilliCream subgraphs, reuse on, mcph 100, 3 rounds:
| | shared | shared + pool | per_worker | per_worker + pool |
|---|---|---|---|---|
| c=500 rps | 5,363 | 5,794 | 5,730 | 5,851 |
| c=500 p99 | 157ms | 128ms | 126ms | 125ms |
| c=500 CPU µs/req | 543 | 403 | 411 | 398 |
| c=500 peak RSS | 362MB | 290MB | 491MB | 288MB |
| c=50 CPU µs/req | 599 | 444 | 456 | 442 |
| c=50 peak RSS | 117MB | 101MB | 123MB | 101MB |
Shared + pool keeps more sockets than shared alone (~850 vs ~190) since they're per worker, yet uses less memory and 26% less CPU: no cross-thread wakeups.

New e2e `http1_pool` runs the bench op 24 times (8 at once) in both modes with the flag on; checked it fails when the pool panics. test_all 2106/2106, e2e 839/840 (only file_watch_works).

### 2026-11-07 — the pool follows `http_client_mode`
`shared` now gets one pool for all workers, `per_worker` keeps one per worker.

ChilliCream subgraphs, reuse on, mcph 100, 3 rounds, medians:
| | shared | shared + one pool | per_worker + pools |
|---|---|---|---|
| c=500 rps | 5,317 | 5,404 | 5,662 |
| c=500 p99 | 152ms | 152ms | 129ms |
| c=500 CPU µs/req | 551 | 523 | 407 |
| c=500 peak RSS | 394MB | 233MB | 296MB |
| c=500 subgraph sockets | ~200 | ~190 | ~830 |
| c=50 CPU µs/req | 618 | 591 | 451 |
| c=50 peak RSS | 117MB | 95MB | 104MB |
One shared pool matches Fusion AOT's memory (233MB) and socket count, but responses wake other threads again: 523 vs 403 µs/req with per-worker pools in shared mode. `per_worker` + pool stays the fast setup.

### 2026-11-07 — ChilliCream benchmark, routers on one core
Their runner (apollo-federation family, HotChocolate subgraphs), 1 warmup + 3 × 60 s runs, 30 s cooldown, no pinning on macOS.
Hive: `http.workers: 1` (main LTO `ea661773`; HEAD LTO `0decd6b8` with slots, entity reuse and the HTTP/1 pool). Under load one thread does ~99% of the router's CPU: ntex runs a current-thread tokio runtime per worker, so hyper's and the pool's connection tasks stay on the worker; only short-lived blocking threads (DNS on connect, startup) sit outside.
Fusion: `DOTNET_PROCESSOR_COUNT=1`, which is not a cap: it still used 1.2–2.4 cores.

| scenario | gateway | req/s | p95 | peak RSS | cores used | CPU µs/req |
|---|---|---|---|---|---|---|
| synthetic | hive-main | 2,132 | 28.0ms | 97MB | 0.93 | 436 |
| | hive-head | 3,650 | 17.5ms | 80MB | 0.93 | 256 |
| | fusion-net-11 | 3,080 | 22.0ms | 286MB | 1.96 | 614 |
| | fusion-aot-11 | 3,169 | 21.4ms | 103MB | 1.93 | 615 |
| burst | hive-main | 1,719 | 286ms | 271MB | 0.95 | 551 |
| | hive-head | 3,205 | 151ms | 225MB | 0.93 | 290 |
| | fusion-net-11 | 3,172 | 149ms | 290MB | 2.33 | 735 |
| | fusion-aot-11 | 3,124 | 151ms | 121MB | 2.38 | 754 |
| real-world | hive-main | 1,557 | 39.4ms | 88MB | 0.78 | 498 |
| | hive-head | 2,501 | 26.5ms | 82MB | 0.67 | 266 |
| | fusion-net-11 | 2,035 | 30.2ms | 265MB | 1.25 | 615 |
| | fusion-aot-11 | 1,928 | 31.9ms | 98MB | 1.22 | 628 |
0 failed requests.

### 2026-11-07 — where parse / write / representations time goes, and projection-in-data probes

Profile (samply, LTO bench build, timed loop only, inline-aware phase split):

| phase | bench_op reuse on (55.6 µs) | bench_op reuse off (59.8 µs) | ct (759 µs) |
|---|---|---|---|
| parse subgraph bodies | 38% | 32% | 76% |
| write client response | 29% | 29% | 23% (a quarter of it memcpy) |
| representations | 24% | 23% | 0% |
| merge | 0.2% | 3.3% | 0% |
| other (plumbing) | 8% | 11% | 1% |

Floors, measured in the same request (non-LTO): bench_op input 80.6 KB in 7 bodies, one NEON
pass over every byte 4.5 µs (parse ~20 µs), copying the 87 KB output once 1.6 µs (write ~17 µs).
ct 3 MB: scan 152 µs (parse ~578), copy 50 µs (write ~180). Both phases cost per value, not per
byte: bench_op parse runs at ~4 GB/s, ct at ~5 GB/s, about simdjson's range.

Writer probes (`WRITE_EXTRA=<mode>:<n>` writes the response n more times per request; patch in
the scratchpad, not committed), µs per write, 3 rounds:

| writer | bench_op | ct |
|---|---|---|
| current (projection plan + slot table) | 16.9 | 180 |
| bitmask: walk each block in slot order, a visible/non-null flag per slot, no plan | 14.1 (−17%) | 153 (−15%) |
| `write_whole` (no projection) | 17.9 | 170 |
| splice: scalars adjacent in the body with exactly `,"key":` between join one memcpy | −5% | −8% |

- The bitmask writer is an upper bound: per-layout flags can't express positions that share a
  layout but select different fields (deviation 1), so it wrote 2% too many bytes on bench_op.
- Memoizing written objects by (object, selection): 0 hits. All 389 objects bench_op writes are
  distinct blocks; the 93% of output bytes that repeat earlier subtrees are equal values, a
  property of the fixture data, not shared objects.
- Splice: only 21% (bench_op) / 25% (ct) of walked scalars join a run, the client order
  interleaves subgraphs and hidden key fields.

Parser probe: one NEON pass for quote / backslash / control / non-ASCII instead of `memchr2`
+ the min/max check pass, for strings past 16 bytes (`PARSE_FUSED=1`): ct 804 → 772 µs (−4%,
3 rounds), bench_op unchanged (short strings take the 8-byte steps). Slot tests pass with it on.

### 2026-11-07 — toward −50%: kept changes (paired, non-LTO, 3 rounds)

Harness: `pipeline_benches`, a baseline binary of HEAD vs the working tree, both checking client
output and subgraph request bodies against the references. ct's body is the 3 MB payload in
the order the fetch asks for (a subgraph answers in that order); the old recorded payload is
also measured ("old order").

| workload | before | after | change |
|---|---|---|---|
| bench_op | 61.6 µs | 54.4 µs | −11.7% |
| bench_op, entity reuse | 56.8 µs | 50.2 µs | −11.6% |
| ct | 790 µs | 477 µs | −39.6% |
| ct, body in the old fetch order | 789 µs | 624 µs | −21% |

Peak bytes and allocations per request are unchanged (ct: 3.5 MB, mostly the output buffer).

What did it, in order:
- **Writer fast loop** (`Writer::simple_fields`, `FieldSlot::SIMPLE`): children with no type
  guard, condition or `__typename` skip those checks, copy scalars inline, and reserve once per
  field for separator + key + short value. bench_op −3.5 µs, then −1 µs.
- **String scan in one NEON pass** (`scan`, `chunk16`): a 16-byte step first, then 32 bytes per
  step, looking for quote / backslash / control and collecting the high bits for the UTF-8
  check. Replaces two SWAR steps + `memchr2` + a min/max check pass. ct −10–15%.
- **Custom scalars in passthrough subtrees** (`Verify::any`): a JSON scalar holding a list or an
  object failed the passthrough check, and the parser re-read the subtree. ct did that 4,500 times
  per request (`attributesRaw.value`).
- **Failed-check memo** (`Arena::failed`): after a passthrough check fails once in a body, the
  rest of the body skips it. A subgraph that orders keys differently costs one failed check, not
  one per list item (ct with mismatched order: 803 → 723 µs at that point).
- **Order-dependent fragment minification** (`minification/selection_id.rs`, planner): fetch
  documents shared one fragment between `{ value locale }` and `{ locale value }`, so the subgraph
  answered in the first order and nothing above those leaves could pass through. Now only
  identical, same-order selection sets share a fragment. ct's fetch document grows by 45 bytes
  (one more fragment); one planner snapshot changed (`requires_fragments.rs`). With it, all of
  ct passes through: the writer is one 3 MB copy.
- **`"key":` in one 16-byte compare** (`ShapeNode::key_colon`, `at_key_colon`) in the passthrough
  check and in `shaped_object`: ct −4.5%, bench_op −0.6 µs.
- **Plain strings straight to the scan** in the passthrough check: ct −6.5%.
- **Fingerprint presence mask over key slots only** (`key_slots`): the representation reads the
  presence of key fields (and whether the object has anything), so only those are hashed, not
  every slot of the layout. bench_op ~−1 µs.

Tried and dropped (no gain or a loss): a 64-byte scan, a 16-byte-only scan, a SWAR 8-byte step
before the NEON one (ct +40 µs), `,"key":` with the comma in `shaped_object` (ct +6 µs),
conditions in the writer fast loop, flags and body slices in `FieldSlot`, a numbers fast path in
the check, a string fast path in `field`.

Not done: zero-copy output (slices of the subgraph body instead of a copy; would save ct's
50 µs copy and the 3 MB buffer) needs `PlanExecutionOutput::body` to stop being a `Vec<u8>`,
which plugin hooks see. bench_op is near per-value floors in parse, write and representations
(every object there is merged from several subgraphs, so nothing passes through).

Tests: `cargo test_all` 2109/2109, `HIVE_SLOT_PIPELINE=1 cargo test_e2e` 839/840 (only
`file_watch_works`), slot tests 5/5 (new: `a_custom_scalar_passes_through_whatever_json_it_holds`,
fails without the fix; `a_list_keeps_its_items_right_after_a_failed_check`), minification test
`selection_sets_in_another_order_get_their_own_fragment`. The non-ARM fallbacks were built and
run on this Mac by swapping the `cfg`s: same outputs, slot tests pass.

Whole router (non-LTO release, `http.workers: 1`, slot pipeline on, HEAD `steps/r-cfg2` vs this
tree, 3 alternating rounds × 15 s; ct through `serve-payload.mjs`, bench op through the bench
subgraphs, 0 validation failures):

| workload | HEAD CPU µs/req | now | req/s |
|---|---|---|---|
| ct, the subgraph answers in the new fetch order | 1,649 | 1,346 (−18%) | 607 → 744 (+23%) |
| ct, the subgraph answers in the old order | 1,640 | 1,472 (−10%) | 610 → 680 (+11%) |
| bench op | 250.7 | 246.4 (−1.7%) | 3,966 → 4,027 |

The pipeline is a part of the router's work: ct also pays for reading and sending 3 MB over
HTTP, the bench op for 7 subgraph calls (pipeline ≈ 55 of ~250 µs).

### 2026-11-07 — chunked output (prototype, dropped)

Tried: the slot writer left raw values past a threshold out of its buffer and the response
went out as pieces (the buffer between them, slices of the subgraph body), as an internal
`QueryPlanExecutionResult::Chunked`, only with no coprocessor and no plugins. Whole router, one
worker, against the commit before it:
- ct, subgraph in the new fetch order (one 3 MB piece): 1,351 → 1,266 µs CPU/req (−6.3%),
  740 → 790 req/s.
- ct in the old order: unchanged at a 64 KB threshold; at 4 KB it got hundreds of pieces and
  was 17% slower (each piece goes through the HTTP writer on its own).
- bench op: unchanged (no value that big).
- Pitfalls: `ntex::util::Bytes` over external storage copies on `slice`/`split`/advance
  (`set_start`/`set_end` copy), so slices had to be cut from `bytes::Bytes` and wrapped one by
  one; ntex still copies the rest of a piece after a partial socket write.

Dropped by the user's call: −6% on one shape of response isn't worth a second body type in the
pipeline and the HTTP layer. `PlanExecutionOutput::body` stays `Vec<u8>`. Patch kept in the
session scratchpad (`opt/chunked-output.patch`).

### 2026-11-07 — coverage: mutations and rewritten projection plans on the slot pipeline

- **Mutations** run on the slot pipeline. Fetch documents may be `mutation`s; like the other
  pipeline, a mutation's subgraph calls aren't deduplicated and its response is `no-store`.
  Subscriptions still fall back.
- **Rewritten projection plans** (authorization nulling fields the client may not see) no
  longer fall back. `SlotPlanCell` keeps the first slot plan plus up to 16 more, one per
  projection plan, matched by content (`ProjectionPlan::same_as`: same fields over the same
  tables), since a rewrite builds a new plan for every request and different client operations
  can share a query plan. Each is compiled with its own projection plan, so a nulled field is
  never inside a passthrough subtree. Past 16 a request compiles one for itself (no fallback).
- e2e with the slot pipeline forced on, counted per request with a temporary tally (not
  committed): 916 queries and 20 mutations on slots. Fallbacks left: subscriptions 418, demand
  control 105, introspection 42, plugins 15, `@fromContext` / context alias templates / rewrites
  39. Nothing for mutations or projection plans.
- Tests: `a_mutation_matches_the_value_pipeline`,
  `a_rewritten_projection_plan_gets_a_slot_plan_of_its_own` (own slot plan, reused by the next
  request with the same rewrite, output equal to the other pipeline's with the field null);
  `cargo test_all` 2111/2111, e2e 839/840 with slots on and off (only `file_watch_works`).
  Bench unchanged.
- **To do: benchmark rewritten requests.** Not measured yet. `router_benches`
  (`authorization_benchmark`) only times `apply_authorization_to_operation`, the rewrite itself,
  which this change doesn't touch. What changed is executing a rewritten request (slot pipeline
  instead of a fallback to the `Value` pipeline). Measure that with a `pipeline_benches` workload
  that applies an authorization-style rewrite to the bench op's projection plan (HEAD vs this
  change), or through the router on `e2e/supergraph-authz.graphql`: the bench op without a token
  (13 errors, most of the tree nulled) and with a `read:price`-only token (only
  `shippingEstimate` nulled). Both routers gave byte-identical responses for the no-token case.

### 2026-11-07 — coverage: demand control on the slot pipeline

- Requests with demand control no longer fall back. The actual-cost walk is generic over the
  data it reads (`demand_control::CostData`): the `Value` tree as before, or `slots::store::
  SlotView` (an object finds a field by its response key in its layout). One walk for both, so
  they count the same.
- `BySubgraph` costs are recorded from each parsed body (before it's merged), `ByResponseShape`
  from the final slots; telemetry, the "actual cost exceeds max cost" warning and the exposed
  cost headers run at the same point as on the other pipeline. Subgraph budgets already worked:
  the slot path shares job preparation and now passes the context to the executor map.
- With demand control on, the parser doesn't pass subtrees through (`parse_body(..,
  passthrough)`): their lists would be bytes the cost can't count.
- e2e `demand_control` (144 tests, incl. the parity suites) with the slot pipeline forced on:
  all pass, and a temporary tally showed all 99 of its demand-controlled requests on slots, no
  fallback. `cargo test_all` 2111/2111, e2e 839/840 with slots on and off (only
  `file_watch_works`). Bench unchanged (no demand control there).
- ponytail: `SlotView::field` searches the layout's keys linearly per field; resolve the cost
  plan to slots if that shows in a profile.

### 2026-11-07 — coverage: introspection on the slot pipeline

- Operations with introspection fields no longer fall back. The introspection part (what
  `partition_operation` splits off) is compiled like a root fetch under a reserved id
  (`compile::INTROSPECTION_FETCH`): its printed document gives the shape and root slots. At run
  time the router's resolved introspection `Value` is written as `{"data":…}`, parsed with that
  shape and merged into the root before the plan runs, as the other pipeline starts from it.
  The writer then does conditions, aliases, `__typename` and null bubbling the usual way. An
  introspection-only operation (no downstream fetches) works the same.
- Different introspection parts over one downstream plan get their own slot plans: their
  projection plans differ (see `SlotPlanCell`), and the cell now compiles with the introspection
  operation too.
- e2e with the slot pipeline forced on and a temporary tally: every introspection request (45)
  on slots; the fallbacks left are 38, all `@fromContext` / context alias templates / rewrites
  (subscriptions and plugins aside). `cargo test_all` 2111/2111, e2e 839/840 with slots on and
  off. Bench unchanged.
- Plugin hooks: TODOs on `OnExecuteEndHookPayload::data` and
  `OnSubgraphExecuteEndHookPayload::execution_result` for a data API of our own instead of
  `Value`, one that keeps the response matching the operation; until then a plugin hooking
  execution keeps the request on the old pipeline.

### 2026-11-07 — coverage: subscriptions on the slot pipeline

- Each subscription event runs on the slot pipeline. The reserved root fetch is now a general
  seed (`compile::Seed`, `SEED_FETCH`): the introspection result, or a subscription event. A
  stream's slot plan is compiled once when it starts (`SlotPlanCell::for_subscription`): the
  per-event plan (the nodes after the subscription's own fetch) plus that fetch's document as
  the seed's shape. Per event, the event's `data` is written back as JSON, parsed with it and
  merged into the root; the entity fetches and the writer run as for a query. An event without
  `data` (only errors, e.g. `SUBGRAPH_GONE` on a callback heartbeat timeout) gives `data: null`,
  like the other pipeline (`http_callback::complete_active_subscription_on_heartbeat_timeout`
  caught the first cut leaving `data` an object).
- e2e with slots forced on and a temporary tally: 422 subscription events, 1061 queries, 26
  mutations on slots; left: 38 queries + 1 mutation (`@fromContext`, context alias templates,
  rewrites) and plugins. `cargo test_all` 2111/2111, e2e 839/840 with slots on and off. Bench
  unchanged.

### 2026-11-07 — coverage: fetch rewrites on the slot pipeline

- Input and output rewrites no longer fall back. The planner makes two kinds, both resolved when
  the slot plan is compiled:
  - **Output `KeyRenamer`** (a field aliased because its type differs between types, e.g.
    `_internal_qp_alias_0: strField`): the shape node keeps the alias as the key it matches in
    the subgraph's JSON, and its value goes to the position and slot of the field's own key
    (`Shape::renamed`). Type conditions on the path don't matter, a shape object has all its
    types' fields. Paths are from `data` for root fetches and from each entity for entity
    fetches and batch aliases, like the other pipeline applies them. Such an object isn't
    passed through (its bytes have the alias).
  - **Input `ValueSetter` of `__typename`** (an `@interfaceObject`/interface entity sent as the
    interface): `AliasProgram::typename`. The representation writer uses the rewritten name for
    the spot when it has a `__typename` that matches, for `__typename`, the key check and the
    fragments, as the other pipeline sees the rewritten entity. Fingerprints hash the original
    name, which is finer and so safe; equal representations still dedupe by their bytes.
  - Input rewrites on a fetch without requires are ignored, as on the other pipeline. Other
    rewrite forms (the planner makes none) stay `Unsupported`.
- e2e with slots forced on and a temporary tally: no rewrite fallbacks; left: `@fromContext`
  (23 compiles) and context alias templates (5). New test
  `an_aliased_field_is_renamed_back_like_the_value_pipeline` (the slot tests' value side now
  applies output rewrites like the executor). `cargo test_all` 2112/2112, e2e 839/840 with
  slots on and off. Bench unchanged.

### 2026-11-07 — coverage: `@fromContext` on entity fetches (slot pipeline)

- An entity fetch (`Flatten`) with context no longer falls back. `Run::prepare_context`:
  - walks the path with each spot's ancestors (`walk_with_ancestors`, the objects a field
    step read from, like `traverse_and_callback`), reads every context variable from its
    sources (`select_value`: so many `Parent`s up, then keys and type conditions; JSON written
    from the slots; null when none) and groups spots by their tuple of values;
  - runs the usual `collect` once per group (`only`: other groups' spots get no entity), so
    representations, dedupe and targets work as for any fetch;
  - sends up to `max_context_groups` groups per request: one group as the plain operation, more
    as the alias template's operation (`AliasedRequests::write`). An aliased response is read
    back with `parse_aliased_body` (each `_entities_<n>` with the `_entities` shape) into one
    list per response key, each group at its offset, padded or cut to what it sent, error paths
    moved the same way (`Run::unaliased`, like `context::unalias_response`); the targets carry
    the same offsets. From there the merge is the usual one.
- Context values are JSON written from slots: an object's keys come in layout order, which can
  differ from the `Value` pipeline's; equal for scalars, and the subgraph reads an input object
  by key anyway.
- e2e `context` (39 tests) all pass with slots forced on, no fallback but the batch fetches
  with context (5, next). `cargo test_all` 2112/2112, e2e 839/840 with slots on and off. Bench
  unchanged.

### 2026-11-07 — coverage: `@fromContext` on batch fetches; no compile fallbacks left

- A `BatchFetch` whose aliases have context no longer falls back (`Run::prepare_batch_context`),
  like `prepare_batch_fetch_job_state` + `prepare_batch_requests`: each alias groups its spots
  (`context_groups` over all its paths), wave `n` sends group `n` of every alias (one with
  fewer groups sends its first group's values and `[]`, aliases sharing a representations
  variable send it once), waves go up to `max_context_groups` per request, aliased when more
  than one; the first wave's alias merges take the later waves' entities at per-alias offsets.
  `Unaliasing` now keeps counts per response key (batch aliases send different numbers).
- e2e with slots forced on and a temporary tally: 1099 queries, 425 subscription events, 27
  mutations on slots, no compile failure. The only fallback left is a router with plugins that
  hook execution (see the TODO on `OnExecuteEndHookPayload::data`). `cargo test_all`
  2112/2112, e2e 839/840 with slots on and off. Bench unchanged.
- The disk filled up during this round (`target/` is 120+ GB); the session's scratch binaries
  and profiles were deleted to make room.

### 2026-11-07 — coverage: plugins on the slot pipeline; nothing falls back in e2e

- A router with plugins no longer sends requests to the old pipeline:
  - `on_execute` start hooks run first, like on the other pipeline: they may change the data
    the plan starts from (it goes into slots when the operation has a seed), the errors and
    the extensions, answer instead, or register end callbacks;
  - subgraph hooks run in the executor map as before; with plugins the executors return the
    parsed response (`raw_response: false`) and the slot path reads `data` from it, so a hook's
    changes count;
  - end callbacks, when any, get the result as a `Value` (the slots written as JSON and read
    back, its bytes kept for the execution) and the response is projected from what they
    leave with `project_by_operation`; without end callbacks the slot writer runs;
  - plugin error handling (status code) and error masking apply on both paths.
- e2e with slots forced on and a temporary tally: 1101 queries, 422 subscription events, 27
  mutations and the 15 plugin requests on slots, **no fallback**. `cargo test_all` 2112/2112,
  e2e 839/840 with slots on and off. Bench unchanged.
- ponytail: data a start hook puts in only reaches slots when the operation has a seed.

### 2026-11-07 — slot compiler: defensive limits that only mirrored nothing

- Plan nodes the slot pipeline didn't know (a `Flatten` over something other than a fetch,
  `Defer`, a nested `Subscription`) were `Unsupported`; the other pipeline doesn't run them
  either (`prepare_job_future` returns no job), so the slot compiler now skips them the same way.
- `MAX_WIDTH` was 512 keys per layout, a performance cut-off; it's now the `u16` slot limit.
  Test `a_wide_object_runs_on_slots` (600 fields, equal to the other pipeline).
- Custom scalar paths under a leaf were `Unsupported`; a leaf keeps everything under it as the
  bytes it came as, which is what a custom scalar needs.
- Left as `Unsupported`, as invariants the planner keeps: a fetch document that doesn't parse,
  no or several operations, an unknown fragment, a key longer than 64 KB, requires without a
  path, context without entities, an entity alias missing from its document, rewrite forms the
  planner doesn't make.
- `target/profiling` (33 GB of this session's profiling builds) was deleted to make room;
  `mbx gc` keeps the in-use target directory.

### 2026-11-07 — fixes from an independent review of today's ports

- With plugins, subgraph `extensions` were dropped (the body rebuilt from the parsed response
  only had `data`): they're applied from the parsed response now. Only real subgraph bytes are
  counted for cache control, and the "malformed response" check only runs on real bodies, like
  the other pipeline.
- Data a plugin's `on_execute` start hook put in was dropped for operations without a seed:
  such a request now compiles a plan of its own with the operation as the seed's shape
  (`Seed::Operation`, which the introspection seed is too), so fetches merge into it.
- A running batch alias whose spots are all null keeps its merge paths (error paths), and the
  partial-response summary flag ignores responses whose `data` is null, like the other pipeline.
- Kept on purpose: a subscription field the planner aliased is renamed back (the other
  pipeline returned null), and spots rewritten to one interface typename with equal keys send one
  representation instead of two (same client response).
- Test `slot_plugins::a_plugin_keeps_subgraph_extensions_and_its_own_data_on_slots` (fails
  without the extensions fix). `cargo test_all` 2113/2113, e2e 840/841 with slots on.
