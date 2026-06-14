# Demand Control in Hive Router

Demand Control protects the router from operations that are too expensive to serve. A single GraphQL request can fan out into thousands of resolver calls and entity fetches across subgraphs, so "requests per second" is a poor proxy for load — one request can be many orders of magnitude more expensive than another. 

Demand Control assigns a numeric **cost** to each operation and lets the router reject (or just measure) operations that exceed a budget.

It implements the [IBM GraphQL Cost Specification](https://ibm.github.io/graphql-specs/cost-spec.html) (`@cost` and `@listSize`), with federation-aware accounting on top.

## What "cost" means

Cost is a dimensionless number, accumulated by walking the operation:

* Composite types (objects, interfaces, unions) contribute a small weight (1 by default).
* Scalars and enums are free (0) by default.
* `@cost(weight:)` overrides the default weight of a field, type, argument, or input field.
* A list field multiplies the cost of its contents by the **list size**.
* `@listSize` tells the router how big a list is expected to be (a fixed `assumedSize`, or a slicing argument like `first`/`last`, or which child fields are the sized ones).
* Input objects passed as arguments add cost proportional to what the client actually sends.
* Mutations has a small flat surcharge.

The two numbers that matter:

* **Estimated cost** — computed *before* execution, from the operation shape and the request's variables. This is what enforcement is based on.
* **Actual cost** — computed *after* execution, from the real data that came back (real list sizes). This is purely for observability: it tells operators how far the estimate was from reality, which is the signal you use to tune your weights and list sizes.

The gap between them (`actual - estimated`, the *delta*) is the single most useful tuning signal: a persistently large delta means the estimates are wrong in one direction or the other.

## Flow of data

Demand Control sits between planning and execution:

```
parse → validate → normalize → plan
                                  │
                                  ▼
                          estimate cost  ── enforce mode & over budget ──▶ reject (no subgraph is contacted)
                                  │
                                  ▼
                               execute  (per-subgraph budgets may skip individual fetches)
                                  │
                                  ▼
                         measure actual cost  ──▶ metrics + response headers
```

The estimate is computed against the **query plan**, not the raw operation. This is what makes it federation-aware: entity fetches, `@requires` round-trips, and work that only exists in the plan (not in the client-visible response) are all accounted for, and cost is attributed per subgraph.

Because the estimate is produced before any subgraph is contacted, an over-budget operation is rejected with **zero** backend traffic — which is the whole point of doing this work up front, based on an estimate.

### Compile once, evaluate per request

The cost of an operation *shape* doesn't change between requests — only the variables do. So the router compiles a cost "formula" once per operation shape and caches it (keyed by the normalized operation, scoped to the current schema). 

At request time it only replays that formula against the request's variables — no full schema traversal is done on the hot path. 

**Repeated and persisted operations therefore pay the compilation cost once.**

## Modes and enforcement

Enforcement happens at two **independent** levels, each with its own `measure`/`enforce` mode:

* **Operation budget** — if the estimated cost of the whole operation exceeds the configured `max`, the request is rejected outright, before execution.
* **Per-subgraph budget** — a subgraph can be given its own, tighter budget. If the part of the plan that targets that subgraph is too expensive, only that subgraph's fetch is skipped; the rest of the plan still runs and the response comes back partial, with an error describing the skipped subgraph. **This leads to partial GraphQL responses**.

Each level's mode decides what happens when its budget is exceeded:

* **`measure`** — the cost is computed, recorded, and exposed, but nothing is rejected. This is the safe way to roll the feature out: watch the real cost distribution in production before turning on enforcement.
* **`enforce`** — over-budget operations (or subgraph fetches) are rejected.

Because the two modes are set separately, you can — for example — enforce per-subgraph budgets while still only measuring the operation-wide budget, or the other way around.

Actual cost is **not** enforced — exceeding the budget after execution is recorded but never turned into a rejection (you can't un-execute a request).

## Errors

When Demand Control rejects something, it returns a structured GraphQL error with a stable code:

* `COST_ESTIMATED_TOO_EXPENSIVE` — the whole operation was rejected before execution because its estimate exceeded the supergraph budget.
* `SUBGRAPH_COST_ESTIMATED_TOO_EXPENSIVE` — a single subgraph fetch was skipped because it exceeded that subgraph's budget; the rest of the response is still produced.
* `COST_INVALID_SLICING_ARGUMENTS` — the operation used a `@listSize` field that requires exactly one slicing argument but provided zero or several, so its cost can't be determined.

These errors carry the relevant cost numbers (estimated and max) in their `extensions`, so a client can correlate the rejection with the configured limit. 

The HTTP status follows the request's content negotiation (the same rules the rest of the pipeline uses for GraphQL-level errors).

## Exposing cost to clients

When asked to, the router exposes the cost of an operation back on the HTTP response as headers — `X-Cost-Estimated`, `X-Cost-Actual`, and `X-Cost-Max` (names are configurable, and each can be enabled independently). 

This is opt-in and off by default; cost is **not** added to the GraphQL `extensions` of successful responses. Errors are the exception: a rejection still carries its cost in the error's `extensions`, because that's where it's actionable.

## Observability

Demand Control is meant to be tuned with data, so the observability is crucial.

**Metrics** — three histograms per operation:

* `cost.estimated`
* `cost.actual`
* `cost.delta` (estimated vs actual; can be negative, so it is a float histogram)

They use cost-shaped bucket boundaries (a count, not a duration or a byte size) and a result-code dimension (`COST_OK`, `COST_ESTIMATED_TOO_EXPENSIVE`, …). 

They are also labelled by operation name, which is the highest-cardinality dimension here — that label can be dropped via the standard metric config when cardinality is a concern.

**Tracing** — the same estimated/actual/delta/result values are attached as attributes on the GraphQL operation span.

**Logs** — a one-line summary of the effective configuration at startup; warnings for configuration that is likely a mistake (enforce mode with a zero budget, or no default list size); warnings when an operation is rejected; and, at debug level, the symbolic cost formula behind a rejection, so an operator can see *why* the estimate came out the way it did.

**Plugins** — cost is also handed to the plugin system through the execution hook: the estimated and max cost are available *before* execution, and the estimated/max/actual triple *after* execution. A plugin can use these to drive its own logic — custom headers, billing, or bespoke logging — without recomputing anything.

## Relationship to persisted documents

The two features are independent and run in a fixed order: a persisted document is resolved to a query first, then cost analysis runs on the result — cost doesn't care whether the operation arrived as a trusted id or as free-form text.

They are complementary, not redundant. Trusted documents gate *which* operations may run; Demand Control gates *how expensive* a given operation is. 

Crucially, an allowlist does not make Demand Control pointless: a trusted operation can still be made arbitrarily expensive through its **variables** (a large `first:`), and that is exactly what request-time cost estimation catches.

## Reference

* IBM GraphQL Cost Specification: <https://ibm.github.io/graphql-specs/cost-spec.html>
* Federation Demand Control docs: <https://www.apollographql.com/docs/graphos/routing/security/demand-control>
