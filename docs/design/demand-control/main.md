# Demand Control in Hive Router

> Status: implemented
> Spec: [IBM GraphQL Cost Specification](https://ibm.github.io/graphql-specs/cost-spec.html)
> Related changeset: [.changeset/feature_demand_control.md](../../../.changeset/feature_demand_control.md)

This document describes the design of the Demand Control feature in Hive Router.
It is intended as a guide for both reviewers and engineers who are new to the
feature: it explains *what* the feature does, *where* it lives in the codebase,
*how* the cost is computed, *how* enforcement works at runtime and *which
trade-offs* were made.

---

## 1. Goals

Demand Control protects the supergraph (and individual subgraphs) from operations
that are expensive to plan or execute. The router implements the IBM Cost
Specification using the `@cost` and `@listSize` directives and adds the
following capabilities on top:

1. **Estimate** the cost of an operation *before* execution (request time) and
   reject it if it exceeds a configured limit.
2. **Measure** the *actual* cost after execution and report `actual` / `delta`
   values for observability.
3. **Per‑subgraph limits** in addition to a global supergraph limit. When a
   per‑subgraph limit is exceeded the rest of the plan still runs; only the
   blocked subgraph is skipped and reported as an error.
4. **Telemetry**: histograms for estimated cost, actual cost and delta plus
  demand-control attributes on the `graphql.operation` span (`cost.estimated`,
  `cost.actual`, `cost.delta`, `cost.result`, `cost.formula_cache_hit`).
5. **Performance**: zero schema traversal during request-time evaluation. The
  estimated formulas and actual-cost traversal plans are *compiled once per
  query shape* and cached; request-time evaluation only replays those compiled
  structures against variables and response data.

Non goals:

- Custom cost functions written in code or VRL.
- Cost budgeting across multiple operations / clients (rate limiting is a
  separate concern).
- Surfacing per‑field cost breakdowns in errors (only per‑subgraph today).

### 1.1 Mental model

If you are new to Demand Control, keep this model in mind while reading the
rest of the document:

1. The router **compiles** a cost plan once per normalized operation shape and
  caches it.
2. Before execution, the router **estimates** the request cost from that plan
  and the coerced variables.
3. In `mode: enforce`, the router can either reject the whole request early
  (supergraph `max`) or mark specific subgraphs as blocked (per-subgraph
  limits).
4. After execution, the router **computes actual cost** either from subgraph
  fetch responses or from the merged response shape, depending on
  `actual_cost_mode`.
5. Estimated cost is used for pre-execution enforcement; actual cost is used
  for observability and, in `mode: enforce`, for post-execution rejection via
  GraphQL errors.

The rest of the document expands those five steps.

### 1.2 How to read this document

Use this order if you are onboarding to Demand Control for the first time:

1. Read section 2 for user-visible behavior and failure modes.
2. Read section 5 to understand the cost model and why `delta` moves.
3. Read section 6 for compile/evaluate architecture and performance.
4. Read section 7 and 8 for enforcement and telemetry semantics.
5. Use sections 11 and 12 only if you need cross-router comparison context.

---

## 2. User‑facing surface

### 2.1 Configuration

Defined in [lib/router-config/src/demand_control.rs](../../../lib/router-config/src/demand_control.rs).

```yaml
demand_control:
  enabled: true
  mode: enforce                   # required: enforce | measure
  include_extension_metadata: true
  strategy:
    static_estimated:
      max: 1000                   # supergraph‑wide limit
      list_size: 10               # default assumed list size
      actual_cost_mode: by_subgraph  # by_subgraph (default) | by_response_shape
      subgraph:
        all:
          max: 500
          list_size: 5
        subgraphs:
          products:
            max: 200
```

Field reference:

- `enabled` — must be `true` for any demand-control processing to occur.
- `mode` — **required**. Controls what happens when a cost limit is exceeded:
  - `enforce`: reject the request (or skip the subgraph) when a limit is
    exceeded.
  - `measure`: never reject; costs are computed, telemetry is emitted and
    response extensions are populated, but no request is blocked. Useful for
    observing real-world cost distribution before enabling enforcement.
- `include_extension_metadata` — when `true`, a `cost` object is appended to
  `extensions` on every response.
- `strategy.static_estimated.max` — required supergraph-wide ceiling (in cost
  units). When `mode: enforce` and the estimated cost exceeds this value, the
  request is rejected before any subgraph is contacted
  (`COST_ESTIMATED_TOO_EXPENSIVE`). When `mode: measure`, a limit can still be
  set so that the correct `result: COST_ESTIMATED_TOO_EXPENSIVE` and `maxCost`
  extension fields are emitted even though the request is not blocked.
- `strategy.static_estimated.list_size` — default assumed list size for fields
  that have no `@listSize` directive.
- `strategy.static_estimated.actual_cost_mode`: Actual cost is **always** computed
  after execution. This setting only controls the computation method:
  - `by_subgraph` (default): sum the actual cost computed per subgraph fetch
    response. Enables the `actualBySubgraph` extension field and is the most
    faithful mode when you care about work done inside subgraph fetches,
    including intermediate entity fetches that do not directly appear in the
    final merged response.
  - `by_response_shape`: traverse the merged response and apply the static cost
    rules. Easier to reason about from the client-visible payload, but it does
    not account for intermediate subgraph work that was required to build that
    payload.
- `strategy.static_estimated.subgraph.all` — inherited by every subgraph unless
  overridden by `strategy.static_estimated.subgraph.subgraphs.<name>`.
- `strategy.static_estimated.subgraph.subgraphs.<name>.max` / `list_size` —
  per-subgraph overrides. Subgraph limits are enforced in `enforce` mode and
  ignored in `measure` mode.

### 2.2 Errors

Three result codes exist in demand-control evaluation. In `mode: enforce`, they
affect the client-visible response as follows:

- `COST_ESTIMATED_TOO_EXPENSIVE`: produced by
  `PipelineError::CostEstimatedTooExpensive` in
  [bin/router/src/pipeline/error.rs](../../../bin/router/src/pipeline/error.rs)
  *before any subgraph is contacted*, when the estimated cost exceeds
  `max`. The HTTP status depends on the negotiated response mode, not on
  `include_extension_metadata`: clients that prefer `application/json` for
  GraphQL errors receive `200 OK`, other non-streaming error responses use
  `400 Bad Request`, and streaming response modes stay on `200 OK`.
- `COST_ACTUAL_TOO_EXPENSIVE`: appended as a GraphQL error to the response
  *after execution* when actual cost exceeds `max`. Actual cost is always
  computed; this error fires when the actual cost is above the configured
  `max`. This is **not** a pipeline abort: the response data is still returned
  alongside the error entry, which carries `maxCost` in its extensions.
- `SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE`: emitted when a per-subgraph limit
  is exceeded. The affected subgraph fetch is skipped; the rest of the plan
  runs normally.

In `mode: measure`, none of the above cause rejection. The `result` field in
`extensions.cost` and the demand-control metrics still reflect the appropriate
code (for example `COST_ESTIMATED_TOO_EXPENSIVE`) so operators can observe
violations without blocking requests.

### 2.3 Response extensions

When `include_extension_metadata: true`, the response carries an `extensions.cost`
entry (the field key on `ExecutionResultExtensions` is `cost`, see
[lib/executor/src/execution/plan.rs](../../../lib/executor/src/execution/plan.rs)):

```jsonc
{
  "extensions": {
    "cost": {
      "estimated": 42,
      "result": "COST_OK",
      "estimatedCostBySubgraph": { "books": 30, "authors": 12 },
      "resultBySubgraph": { "books": "COST_OK", "authors": "COST_OK" },
      "estimatedFormulaBySubgraph": { "books": "(1 + ($limit * (1 + 1)))" },
      "maxCost": 1000,
      "formulaCacheHit": true,
      "actual": 30,
      "delta": -12,
      "actualCostBySubgraph": { "books": 30 }
    }
  }
}
```

The serialisable struct is `DemandControlResponseExtensions` in
[lib/executor/src/execution/demand_control.rs](../../../lib/executor/src/execution/demand_control.rs).
`actual` and `delta` are always present. `actualCostBySubgraph` is present
when `actual_cost_mode: by_subgraph`. `resultBySubgraph` lists every
subgraph that participated in the plan with its result code (`COST_OK` or
`SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE` for subgraphs that exceeded their
per-subgraph max).

The human‑readable `estimatedFormulaBySubgraph` field is purely diagnostic:
it is the `Display` implementation of the compiled `CostExpr` and is meant to
help operators understand *why* a query has the cost it has.

### 2.4 Operator quick reference

Use this section during rollout, incidents, or threshold tuning.

#### Mode and threshold decision flow

1. Start with `mode: measure` in production traffic.
2. Enable `include_extension_metadata: true` for a limited period so you can
  inspect `extensions.cost` directly.
3. Review `cost.estimated`, `cost.actual`, and `cost.delta` distributions.
4. Set global `max` high enough to avoid obvious false positives.
5. Add `subgraph.all.max` and optional per-subgraph overrides only after you
  identify heavy subgraphs from telemetry.
6. Switch to `mode: enforce` after the above looks stable.

#### What fails when

| Signal | Stage | Effect |
|---|---|---|
| `COST_ESTIMATED_TOO_EXPENSIVE` | Before execution | Whole request is rejected in `mode: enforce` |
| `SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE` | During execution planning/execution boundary | Only that subgraph fetch is blocked; rest of plan continues |
| `COST_ACTUAL_TOO_EXPENSIVE` | After execution | Response keeps `data` and includes a GraphQL error in `mode: enforce` |

In `mode: measure`, none of the above reject the request; they remain
observable through result codes and telemetry.

#### Fast telemetry triage

1. Check `cost.result` cardinality first to see whether violations are mostly
  estimated or actual.
2. If estimated violations dominate, tune `@listSize`, global `list_size`, and
  per-subgraph list-size defaults.
3. If actual violations dominate with small estimated values, inspect
  `actual_cost_mode` and compare `by_subgraph` versus `by_response_shape`
  expectations for your traffic.
4. If subgraph blocks spike, review per-subgraph limits and top contributors in
  `bySubgraph` and `actualBySubgraph`.
5. Use `delta` direction as a hint:
  - Mostly negative: estimates are conservative.
  - Mostly positive: estimates are too optimistic; tighten assumptions.

#### Minimum rollout checklist

- Demand control enabled with explicit `mode`.
- Global `max` configured.
- `actual_cost_mode` chosen deliberately (`by_subgraph` is usually best for
  operator visibility into real fetch work).
- Baseline telemetry collected in `mode: measure` before enforcement.
- Alerting defined for sustained non-`COST_OK` result ratios.

---

## 3. Where the code lives

| Concern | File |
|---|---|
| `@cost` / `@listSize` schema directive parsing | [lib/query-planner/src/federation_spec/demand_control.rs](../../../lib/query-planner/src/federation_spec/demand_control.rs) |
| Configuration types | [lib/router-config/src/demand_control.rs](../../../lib/router-config/src/demand_control.rs) |
| Plan‑time compilation + request‑time evaluation | [bin/router/src/pipeline/demand_control.rs](../../../bin/router/src/pipeline/demand_control.rs) |
| Execution context, response extensions, actual‑cost evaluation | [lib/executor/src/execution/demand_control.rs](../../../lib/executor/src/execution/demand_control.rs) |
| Cache plumbing (`demand_control_formula_cache`) | [bin/router/src/cache_state.rs](../../../bin/router/src/cache_state.rs), [bin/router/src/schema_state.rs](../../../bin/router/src/schema_state.rs) |
| Pipeline integration | [bin/router/src/pipeline/execution.rs](../../../bin/router/src/pipeline/execution.rs) |
| Metrics | [lib/internal/src/telemetry/metrics/demand_control_metrics.rs](../../../lib/internal/src/telemetry/metrics/demand_control_metrics.rs) |
| Span attributes | [lib/internal/src/telemetry/traces/spans/graphql.rs](../../../lib/internal/src/telemetry/traces/spans/graphql.rs) (`GraphQLOperationSpan::record_demand_control`) |
| E2E tests (per-concern) | [e2e/src/demand_control/](../../../e2e/src/demand_control/) |
| E2E tests (fixture parity) | [e2e/src/demand_control_parity/](../../../e2e/src/demand_control_parity/), [e2e/fixtures/demand_control/](../../../e2e/fixtures/demand_control/) |
| Test supergraph | [e2e/supergraph_demand_control.graphql](../../../e2e/supergraph_demand_control.graphql) |

---

## 4. Pipeline integration

Demand control sits between *query planning* and *execution*:

```
parse → validate → normalize → plan → coerce variables ─┐
                                                        ▼
                                            evaluate_demand_control()
                                                        │
                                ┌───────────────────────┴──────────────────────┐
                                ▼                                              ▼
                  estimated > max?                               DemandControlExecutionContext
                                │                                              │
                                ▼                                              ▼
                  abort with COST_ESTIMATED_TOO_EXPENSIVE         execute query plan
                                                                  (skipping any subgraphs over their limit in `mode: enforce`)
                                                                               │
                                                                               ▼
                                                       demand_control_actual_cost()
                                                                               │
                                                                               ▼
                                              actual > max?      → append COST_ACTUAL_TOO_EXPENSIVE
                                                                  to response `errors` (data still returned)
                                                                               │
                                                                               ▼
                                                       attach `extensions.cost`
```

Entry point: `evaluate_demand_control` in
[bin/router/src/pipeline/demand_control.rs](../../../bin/router/src/pipeline/demand_control.rs).
The returned `DemandControlExecutionContext` is threaded through
[bin/router/src/pipeline/execution.rs](../../../bin/router/src/pipeline/execution.rs) so the
executor can:

- Refuse to call subgraphs listed in `subgraphs_over_limit` when
  `mode: enforce` is configured. In `mode: measure` the same set is
  retained so the per-subgraph result code can still be reported in the
  response extension, but the calls are not blocked.
- Run actual‑cost evaluation in the configured actual-cost mode after execution.
- Gate actual-cost rejection on `mode: enforce`; in `mode: measure` the
  `COST_ACTUAL_TOO_EXPENSIVE` result code is still recorded in telemetry and
  extensions but no error is appended to the response.
- Attach response extensions when `include_extension_metadata: true`.

---

## 5. Cost model

At a high level, the cost model has two phases:

- **Estimated cost**: computed before execution from the query plan, schema
  directives and coerced variables.
- **Actual cost**: computed after execution from either subgraph responses or
  the merged response, depending on `actual_cost_mode`.

Both phases use the same static weights from the schema. The main difference is
which runtime data source they use to resolve cardinality and shape.

> Key takeaway: estimated cost is computed before execution for enforcement,
> while actual cost is computed after execution for observability and
> post-execution enforcement behavior.

### 5.1 Static rules (estimated cost)

Following the IBM cost spec:

| Construct | Cost contribution |
|---|---|
| Query operation | `0` |
| Mutation operation | `10` |
| Subscription operation | `0` |
| Composite return type (object/interface/union) | `1` per occurrence (overridable by `@cost` on the type) |
| Scalar / enum return type | `0` (overridable by `@cost`) |
| Field with `@cost(weight: N)` | adds `N` |
| Argument with `@cost(weight: N)` (when supplied) | adds `N` |
| Input object value | sum of `@cost` weights for present input fields, recursively |
| List field | per‑item cost is multiplied by *list size* |

List size is determined from `@listSize`:

1. `assumedSize: N`: list size is the constant `N`.
2. `slicingArguments: ["limit"]`: list size is resolved from request variables
   or literals.
   - When the argument is *integer‑typed*, its value is used as the list size.
   - When the argument is *list‑typed* (e.g. `ids: [ID!]!`), the **length of
     the supplied list** is used (matching the IBM cost spec). This works for
     both literal list arguments and lists supplied via variables, and is
     implemented in `resolve_integer_value` / `resolve_integer_from_json_value`
     in [bin/router/src/pipeline/demand_control.rs](../../../bin/router/src/pipeline/demand_control.rs).
   - `requireOneSlicingArgument: true` (default): exactly one of the listed
     arguments must be present, otherwise fall back to the configured default.
   - `false`: take `max(...)` of the present ones.
3. `sizedFields`: the size override is propagated to a *child* field rather
   than the field carrying the directive (e.g. Relay‑style `Connection.edges`).
4. None of the above: falls back to `strategy.static_estimated.list_size` (or per‑subgraph
  override). When this is unset, the resulting estimate can be significantly
  lower than runtime cardinality.

`@skip` / `@include` are honoured: an excluded field contributes `0`.

### 5.2 Actual cost

Two implementation strategies (`actual_cost_mode`):

| Mode | Runtime input | What it captures well | Main limitation |
|---|---|---|---|
| `by_subgraph` | Each subgraph fetch response | Work actually performed by subgraph fetches, including `_entities` fetches and aliased BatchFetch responses | Requires compiled plans per fetch hash and per-subgraph accumulation |
| `by_response_shape` | Final merged response | Cost implied by the client-visible payload | Does not include intermediate subgraph work that disappears from the final response |

- **`by_response_shape`**: the merged response is evaluated with
  `estimate_actual_response_shape_cost_with_compiled_plan`, using a precompiled
  `CompiledResponseShapeActualCostPlan`.
- **`by_subgraph`**: each subgraph fetch response is evaluated with
  `estimate_actual_subgraph_response_cost_with_compiled_plan`, using a
  precompiled `CompiledSubgraphActualCostPlan` keyed by fetch operation hash.
  Per-subgraph costs are then summed.

For `_entities` responses, subgraph actual-cost compilation uses an entity-group
model:

- top-level `_entities` selections are recognized whether plain
  (`_entities`) or aliased (`_e0: _entities`, `_e1: _entities`), so BatchFetch
  and FlattenFetch are both handled.
- each group is keyed by response key (field name or alias), and stores
  per-type compiled plans from inline-fragment type conditions.
- during evaluation, if `__typename` is present it is used directly; otherwise
  the evaluator uses a single-entry shortcut when exactly one type plan exists
  for that group.
- for non-entity selection sets, inline fragments remain active when the parent
  type is already known at compile time; missing `__typename` does not disable
  a fragment whose type condition is identical to the known parent type.
- nested inline-fragment type conditions inside an entity selection are not
  treated as additional root entity types; only root-level entity type
  conditions determine `_entities` group dispatch.

In both modes `delta = actual - estimated` and is reported as a histogram and
in the response extension.

The relationship between estimated and actual cost depends on the chosen actual
mode:

- In **`by_subgraph`**, the two values usually differ because estimated cost is
  based on assumed cardinality while actual cost uses the real fetch responses.
  In that mode, list length and entity fan-out are the main sources of delta.
- In **`by_response_shape`**, the two values can also differ because actual cost
  is derived from the final merged response rather than from the intermediate
  fetch work. That means the actual value may be lower than the estimate even
  when list cardinalities match, simply because some execution work does not
  survive into the final response shape.

Even with that distinction, several important inputs stay identical between
estimate time and actual time:

- Both phases use the same schema-derived cost weights from
  `SupergraphState`, but those weights are compiled into the estimated and
  actual cost plans ahead of request-time evaluation. Request-time evaluation
  does not perform schema traversal or per-request recompilation.
- The same coerced variables payload is used in both phases, so `@skip` /
  `@include` conditions (and any other variable‑driven branches such as
  `CostExpr::Cond` or `InputArgCost`) resolve **identically** at estimate
  time and at actual time. A field excluded by `@skip(if: true)` contributes
  `0` to both, never just one side.
- Null field values contribute base field cost only (no per-item or child
  contribution), mirroring the static estimator behavior.

In practice:

- `delta` is often *negative* when real list lengths are smaller than what
  `assumedSize` / `slicingArguments` / `list_size` told the estimator.
- `delta` can be *positive* when a subgraph returned more items than the
  estimator assumed.
- In `by_response_shape`, `delta` can also be more negative than expected from
  list lengths alone because some intermediate execution work is not visible in
  the merged response.

This makes `delta` a useful tuning signal, but it should be interpreted in the
context of the selected actual-cost mode.

---

## 6. Compile / evaluate split

The hot path is *evaluation*; planning is amortised across many requests with
the same shape. We therefore split the work into compile-time plan building and
request-time evaluation for **both** estimated cost and actual cost.

> Key takeaway: compile once per normalized operation shape, then replay
> compiled estimated and actual plans per request.

### 6.1 Compile phase

Triggered the first time a normalized operation hash is seen. Inputs:

- The `QueryPlan` for the operation.
- The `SupergraphState` (for type/field/argument cost lookups).
- The `DemandControlConfig`.

Output: `DemandControlFormulaPlan`, which contains both the estimated-cost
formula tree and the precompiled actual-cost plan:

- `root: FormulaPlanNode` mirroring the query plan structure
  (`Fetch | Aggregate | Condition`).
- `formula_by_subgraph`: `Display` strings for diagnostics.
- `actual_cost_plan`: either:
  - `CompiledActualCostPlan::BySubgraph`, a map of fetch-hash to
    `CompiledSubgraphActualCostPlan`.
  - `CompiledActualCostPlan::ByResponseShape`, a single
    `CompiledResponseShapeActualCostPlan` for the operation.

Each `FormulaPlanNode::Fetch` contains a **`CostExpr`** AST:

```rust
enum CostExpr {
    Const(u64),                                 // compile‑time constant
    Add(Vec<CostExpr>),                         // sum
    Mul(Box<CostExpr>, Box<CostExpr>),          // list_size × per_item
    Cond { variable, if_true, if_false },       // @skip / @include
    ListSize { args, require_one, default },    // resolved at request time
    InputArgCost { value, value_type },         // input‑object cost via vars
}
```

Constructors aggressively constant‑fold (`add_nonzero`, `mul`) so the runtime
expression is as small as possible. `CostExpr` implements `Display`, which is
what `estimatedFormulaBySubgraph` exposes.

The compile phase also precomputes the traversal structure needed for actual
cost evaluation:

- In `by_subgraph`, each fetch is compiled once into a
  `CompiledSubgraphActualCostPlan`, including `_entities` dispatch metadata and
  field/type costs needed to price that fetch response later.
- In `by_response_shape`, the operation selection set is compiled once into a
  `CompiledResponseShapeActualCostPlan` that can be replayed against the final
  merged response.

### 6.2 Evaluate phase

Evaluation happens in two places:

- `evaluate_formula_plan` walks the compiled `FormulaPlanNode` tree,
  evaluating each fetch’s `CostExpr` against the request’s
  `CoerceVariablesPayload` to produce the **estimated** cost.
- After execution, `demand_control_actual_cost` evaluates the precompiled
  `actual_cost_plan` to produce the **actual** cost, either by summing
  per-subgraph fetch costs or by replaying the compiled response-shape plan
  against the merged response.

For estimated cost, `Condition` nodes inspect the runtime boolean variable and
recurse into the matching branch (or neither). All arithmetic uses
`saturating_*` to avoid overflow on hostile inputs (e.g. user‑controlled list
sizes).

The same coerced variables payload that the executor will use is reused, so
estimation cannot disagree with the values passed to subgraphs.

For actual cost, request-time evaluation uses the compiled actual-cost plans;
it does not re-read schema directive metadata or rebuild traversal state per
request.

### 6.3 Caching

Plans are cached in `SchemaState::demand_control_formula_cache` keyed by the
*normalized operation hash* (`Cache<u64, Arc<DemandControlFormulaPlan>>`,
capacity 1000). The cache is invalidated whenever the schema changes, alongside
the other per‑schema caches in
[bin/router/src/cache_state.rs](../../../bin/router/src/cache_state.rs).
Hit/miss rates are reported via the existing cache metrics
(`metrics.cache.demand_control_formula`).

---

## 7. Per‑subgraph enforcement

`subgraphs_over_limit(config, evaluation)` computes the set of subgraphs whose
aggregated estimated cost exceeds their effective limit
(`subgraphs.<name>.max` if present, otherwise `subgraph.all.max`).
When a subgraph appears in multiple plan nodes (entity lookups, nested fetches,
conditional branches), the per‑subgraph cost is *summed across the whole plan*
(see `collect_estimated_formulas` and the per‑subgraph accumulation in
`evaluate_formula_plan_node`) and the limit is enforced against that total.

The set is attached to the `DemandControlExecutionContext` and consulted by
`SubgraphExecutorMap::execute` in
[lib/executor/src/executors/map.rs](../../../lib/executor/src/executors/map.rs):
in `mode: enforce`, any fetch to a subgraph in this set short‑circuits with
`SubgraphExecutorError::CostEstimatedTooExpensive`, which surfaces in the
GraphQL response as an error with code `SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE`
(distinct from the supergraph‑wide `COST_ESTIMATED_TOO_EXPENSIVE`) while the
rest of the plan continues to run. In `mode: measure` the same set is still
produced and surfaced via `result_by_subgraph` in `extensions.cost`, but
fetches are not blocked.

Note that the supergraph‑wide `max` is enforced *before* execution and
fails the whole request (in `mode: enforce`), while subgraph limits are
*partial* and let other subgraphs proceed. This asymmetry is intentional and
called out in the config docs. In `mode: measure`, neither the supergraph
nor per-subgraph limits trigger rejection — `subgraphs_over_limit` is
still computed and reported, but `DemandControlMode::Measure` is forwarded
to both the per-subgraph gate and the actual-cost enforcement path so no
rejection actually happens.

---

## 8. Telemetry

Three histograms in `DemandControlMetrics`
([demand_control_metrics.rs](../../../lib/internal/src/telemetry/metrics/demand_control_metrics.rs)):

- `cost.estimated` (`u64`, unit `By`)
- `cost.actual` (`u64`, unit `By`)
- `cost.delta` (`f64`, unit `By`)

All three are labelled with `cost.result` (`COST_OK`,
`COST_ESTIMATED_TOO_EXPENSIVE`, `COST_ACTUAL_TOO_EXPENSIVE`) and, when
available, `graphql.operation.name`.

Demand-control telemetry is recorded on `graphql.operation` via
`GraphQLOperationSpan::record_demand_control` (see
[graphql.rs](../../../lib/internal/src/telemetry/traces/spans/graphql.rs)).
Recorded attributes are: `cost.estimated`, `cost.actual`, `cost.delta`,
`cost.result`, and `cost.formula_cache_hit`.

---

## 9. Test coverage

End‑to‑end tests live in
[e2e/src/demand_control/](../../../e2e/src/demand_control/) and are split by
concern across a small set of files:

- [`estimator.rs`](../../../e2e/src/demand_control/estimator.rs) — `@cost` /
  `@listSize` rules and other estimator behaviour.
- [`enforcement.rs`](../../../e2e/src/demand_control/enforcement.rs) —
  `mode: enforce` vs. `mode: measure` and the supergraph-wide `max`.
- [`actual_cost.rs`](../../../e2e/src/demand_control/actual_cost.rs) —
  `by_subgraph` and `by_response_shape` actual-cost modes, batch fetches,
  entity fetches, deltas.
- [`subgraph_budgets.rs`](../../../e2e/src/demand_control/subgraph_budgets.rs)
  — per-subgraph `max` and named-subgraph overrides.
- [`extensions.rs`](../../../e2e/src/demand_control/extensions.rs) —
  `extensions.cost` shape, `formulaCacheHit`, `estimatedFormulaBySubgraph`.
- [`metrics.rs`](../../../e2e/src/demand_control/metrics.rs) — OTel
  histograms and `graphql.operation` span attributes.

The fixtures live at
[e2e/supergraph_demand_control.graphql](../../../e2e/supergraph_demand_control.graphql),
which contains a full menagerie of `@cost` / `@listSize` placements
(field, argument, type, enum, scalar; assumed size, single slicing argument,
multiple slicing arguments, `sizedFields`, dotted paths into input objects).

In addition to the per-concern suite above, a parity suite at
[e2e/src/demand_control_parity/](../../../e2e/src/demand_control_parity/)
runs a curated set of fixtures (under
[e2e/fixtures/demand_control/](../../../e2e/fixtures/demand_control/)) end-to-end
against the canned-mock subgraph harness in
[e2e/src/testkit/mock_subgraphs.rs](../../../e2e/src/testkit/mock_subgraphs.rs)
and asserts on the full `extensions.cost` payload (top-level result,
per-subgraph result codes, per-subgraph call counts). The suite is split by
scenario:

- [`estimated_cost.rs`](../../../e2e/src/demand_control_parity/estimated_cost.rs)
  — pure estimated-cost numbers vs. curated reference values.
- [`within_max.rs`](../../../e2e/src/demand_control_parity/within_max.rs) —
  requests that fit under the supergraph-wide `max`.
- [`exceeds_max.rs`](../../../e2e/src/demand_control_parity/exceeds_max.rs)
  and
  [`exceeds_max_with_subgraph_config.rs`](../../../e2e/src/demand_control_parity/exceeds_max_with_subgraph_config.rs)
  — supergraph-wide rejection paths.
- [`measure_mode.rs`](../../../e2e/src/demand_control_parity/measure_mode.rs)
  — `mode: measure` parity.
- [`subgraph_budget.rs`](../../../e2e/src/demand_control_parity/subgraph_budget.rs)
  — per-subgraph budget exhaustion that leaves the rest of the plan
  running.
- [`actual_cost_modes.rs`](../../../e2e/src/demand_control_parity/actual_cost_modes.rs)
  — `by_subgraph` vs. `by_response_shape` actual-cost modes.
- [`list_size_inheritance.rs`](../../../e2e/src/demand_control_parity/list_size_inheritance.rs)
  — `list_size` inheritance from `subgraph.all` and per-subgraph overrides.

Categories covered:

- Baseline cost without any directives.
- `@cost` on type / field / argument / enum / scalar / input field.
- `@listSize` with `assumedSize`, single and multiple slicing arguments,
  `requireOneSlicingArgument`, `sizedFields`, deeply nested slicing arg paths.
- Negative literal slicing arguments (must be clamped, not cast).
- Variables with `@skip` / `@include`.
- Supergraph‑level `max` enforcement vs. per‑subgraph blocking.
- Both actual‑cost modes and the resulting `actual` / `delta` /
  `actualCostBySubgraph` extension fields.
- Per-subgraph `resultBySubgraph` reporting (`COST_OK` /
  `SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE`).
- Telemetry assertions on the three histograms and demand-control attributes on
  `graphql.operation` spans.

---

## 10. Trade‑offs and notes for reviewers

- **No partial cost rejection at the supergraph level.** A supergraph
  `max` failure aborts the request entirely, even if only one branch is
  expensive. Per‑subgraph limits exist exactly to support a more granular
  policy.
- **Unset list-size behavior can under-estimate.** If no `@listSize`, no
  per-subgraph `list_size`, and no global `strategy.static_estimated.list_size`
  are provided, list-heavy operations may be under-estimated. Operators should
  set explicit defaults for safer enforcement.
- **`@cost(weight: N)` on input fields** is consumed only when the field is
  actually present in the request; this is implemented through
  `CostExpr::InputArgCost`, evaluated per request against the coerced
  variables payload.
- **Mutations carry a flat `+10`** as per the spec. Subscriptions are treated
  like queries (`+0`); the per‑event cost is already accounted for by the
  selection set.
- **Saturating arithmetic everywhere.** Hostile inputs (huge list sizes,
  pathological nested input objects) cannot cause overflow; the worst case is
  `u64::MAX`, which any sensible `max` will reject.
- **No schema traversal during request-time evaluation.** The estimated-cost
  `CostExpr` tree closes over precomputed type / field / argument costs at
  compile time, and the actual-cost traversal plans likewise embed the field /
  type costs they need before request-time evaluation begins. This is the main
  reason for the compile / evaluate split.
- **Diagnostic formula strings are only produced when
  `include_extension_metadata: true`.** They are derived from the compiled
  `CostExpr` so they cannot drift from what is actually evaluated.

---

## 11. Comparison with Apollo Router

This section calls out where Hive Router and Apollo Router converge or
deliberately diverge. Reviewers familiar with Apollo's
[Demand Control documentation](https://www.apollographql.com/docs/graphos/routing/security/demand-control)
should use this as the diff.

If you are new to both systems, the short version is:

- The two routers agree on the IBM cost model and the main configuration shape.
- Hive adds stronger client-visible actual-cost enforcement.
- Hive reports blocked subgraphs as explicit GraphQL errors instead of treating
  them as silent null-like results.
- Hive exposes cost data primarily through GraphQL extensions and built-in
  telemetry rather than through Apollo-style context keys and telemetry
  selectors.

### Same

- Cost specification (IBM): default operation/type weights, `@cost(weight: Int!)`
  on objects, scalars, enums, fields, arguments and input fields, and the full
  `@listSize` directive (`assumedSize`, `slicingArguments`, including
  list‑typed arguments where the length of the list is used, `sizedFields`,
  `requireOneSlicingArgument`, dotted nested paths).
- Metric names (`cost.estimated`, `cost.actual`, `cost.delta`) and result codes
  (`COST_OK`, `COST_ESTIMATED_TOO_EXPENSIVE`, `COST_ACTUAL_TOO_EXPENSIVE`).
- Two actual‑cost evaluation modes: `by_subgraph` and `by_response_shape`.
- Subgraph‑level demand control with `subgraph.all` defaults overridden by
  `subgraph.subgraphs.<name>`, costs aggregated across multiple fetches to the
  same subgraph in a single plan, and partial execution when a subgraph limit
  is exceeded.
- The semantic that estimated cost is computed before any subgraph is hit and
  rejects the request when over the supergraph‑wide limit.

### Intentionally different

| Topic | Apollo Router | Hive Router |
|---|---|---|
| Configuration shape | Same nested `static_estimated` strategy plus `mode: measure \| enforce`. | Same overall shape and semantics. |
| Default `actual_cost` mode | `by_subgraph` is the default. | `by_subgraph` is also the default. |
| Subgraph block behaviour | A blocked subgraph is composed more like a null result for that branch. | A blocked subgraph surfaces as an explicit GraphQL error with code `SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE`, while the rest of the plan continues. |
| Actual‑cost overrun | Actual cost is recorded, but the static estimated strategy does not reject on actual-cost overflow. | Actual cost can still trigger a client-visible `COST_ACTUAL_TOO_EXPENSIVE` error after execution. |
| Programmatic access | Cost values are exposed through Apollo-specific context keys consumable by Rhai/coprocessors. | Cost values are exposed through `extensions.cost` and built-in telemetry. |
| Telemetry customisation | Supports selector-based telemetry customization in YAML. | Exposes fixed demand-control metrics and a dedicated span, but no selector DSL. |
| Diagnostic formula strings | Not provided. | Exposes `estimatedFormulaBySubgraph` for human-readable estimated formulas. |
| Formula cache observability | Not surfaced directly. | Exposes `formulaCacheHit` and a dedicated cache metric. |

Reference notes:

- Apollo config shape and mode semantics: [`docs/shared/config/demand_control.mdx`](https://github.com/apollographql/router/blob/dev/docs/shared/config/demand_control.mdx#L0-L17), [`apollo-router/src/plugins/demand_control/mod.rs`](https://github.com/apollographql/router/blob/dev/apollo-router/src/plugins/demand_control/mod.rs#L174)
- Apollo default actual-cost mode: [`apollo-router/src/plugins/demand_control/mod.rs`](https://github.com/apollographql/router/blob/dev/apollo-router/src/plugins/demand_control/mod.rs#L107), [PR #8827](https://github.com/apollographql/router/pull/8827)
- Apollo blocked-subgraph behavior: [`docs/source/routing/security/demand-control.mdx`](https://github.com/apollographql/router/blob/dev/docs/source/routing/security/demand-control.mdx#L342), [`strategy/static_estimated.rs`](https://github.com/apollographql/router/blob/dev/apollo-router/src/plugins/demand_control/strategy/static_estimated.rs#L60), [`demand_control/mod.rs`](https://github.com/apollographql/router/blob/dev/apollo-router/src/plugins/demand_control/mod.rs#L658)
- Apollo actual-cost recording path: [`apollo-router/src/plugins/demand_control/strategy/static_estimated.rs`](https://github.com/apollographql/router/blob/dev/apollo-router/src/plugins/demand_control/strategy/static_estimated.rs#L114)
- Apollo context keys and telemetry selectors: [`plugins/rhai/engine/mod.rs`](https://github.com/apollographql/router/blob/dev/apollo-router/src/plugins/rhai/engine/mod.rs), [`docs/source/routing/customization/coprocessor/reference.mdx`](https://github.com/apollographql/router/blob/dev/docs/source/routing/customization/coprocessor/reference.mdx#L395), [`plugins/telemetry/config_new/cost/mod.rs`](https://github.com/apollographql/router/blob/dev/apollo-router/src/plugins/telemetry/config_new/cost/mod.rs)

### Apollo features not implemented today

The following Apollo features were considered and intentionally left out of
the initial scope. None of them are blocking for parity with the cost spec
itself; they can be added incrementally.

- **Programmatic context‑key access** (`APOLLO_COST_*`). Plugins can already
  read `DemandControlExecutionContext`, but there is no stable pre‑defined
  context‑key contract.
- **Telemetry selector DSL** for cost values in custom instruments / spans /
  events. Operators get the three default histograms and the dedicated span,
  but cannot mix cost values into custom instruments via YAML.

## 12. Comparison with Cosmo Router

Cosmo Router (WunderGraph) also implements the IBM Cost Specification, but with
a much smaller surface than Apollo or Hive. The reference is Cosmo's
[Cost Control documentation](https://cosmo-docs.wundergraph.com/router/security/cost-control)
and the implementation in
[`router/pkg/config/config.go`](https://github.com/wundergraph/cosmo/blob/main/router/pkg/config/config.go#L515)
plus
[`router/core/operation_processor.go`](https://github.com/wundergraph/cosmo/blob/main/router/core/operation_processor.go#L1411).

The short version is:

- Cosmo implements the same IBM cost directives, but with a smaller feature
  surface.
- Cosmo focuses on one global estimated-cost limit, while Hive also supports
  per-subgraph limits.
- Cosmo exposes cost mainly through headers and metrics; Hive exposes a richer
  GraphQL `extensions.cost` payload.
- Hive also enforces actual-cost overruns; Cosmo does not.

### Same

- Implements the IBM Cost Specification: `@cost(weight)` on fields, arguments,
  input fields, types; `@listSize(assumedSize, slicingArguments, sizedFields,
  requireOneSlicingArgument)`; default operation/type weights.
- Estimated cost is computed before any subgraph fetch; in `enforce` mode the
  request is rejected before any subgraph is hit.
- Exposes both estimated and actual cost values (Cosmo via the
  `X-WG-Cost-Estimated` / `X-WG-Cost-Actual` response headers and OTel
  histograms; Hive via `extensions.cost` and OTel histograms).
- `measure` semantics: cost is recorded but never causes rejection.

### Intentionally different

| Topic | Cosmo Router | Hive Router |
|---|---|---|
| Configuration shape | Flat cost-control config with one main estimated-cost limit and optional header exposure. | Nested demand-control config with global, per-subgraph, and actual-cost settings. |
| Subgraph‑level limits | Not supported. | Supported, including inherited defaults and per-subgraph overrides. |
| Rejection error format | Plain HTTP `400` with a free-form message and no GraphQL error code. | Structured GraphQL errors with stable demand-control codes and `maxCost` metadata. |
| Actual‑cost overrun | Actual cost is exposed, but not enforced. | Actual cost is exposed and can also trigger `COST_ACTUAL_TOO_EXPENSIVE`. |
| Programmatic access | Exposes estimated cost to Go modules, but not actual/per-subgraph cost. | Exposes richer cost data through `extensions.cost` and the execution context. |
| Diagnostic formula strings | Not provided. | Exposes `estimatedFormulaBySubgraph`. |
| Telemetry | Histograms for estimated and actual cost only. | Histograms for estimated, actual, and delta cost plus result labeling and a dedicated span. |

Reference notes:

- Cosmo config surface: [`router/pkg/config/config.go`](https://github.com/wundergraph/cosmo/blob/main/router/pkg/config/config.go#L515)
- Cosmo rejection behavior: [`router/core/operation_processor.go`](https://github.com/wundergraph/cosmo/blob/main/router/core/operation_processor.go#L1432)