---
hive-router: patch
hive-router-plan-executor: patch
---

# Record subgraph execution errors on the `graphql.subgraph.operation` span

Errors raised while preparing or executing a subgraph fetch
(`PlanExecutionError`) are now attached to the corresponding
`graphql.subgraph.operation` span instead of only surfacing on the
top-level `graphql.operation` span via the response-error pipeline.

For each failing fetch the span now carries:
- `hive.graphql.error.count = 1`,
- `hive.graphql.error.codes` set to the error code (e.g.
  `SUBGRAPH_REQUEST_TIMEOUT`, `HEADER_PROPAGATION_FAILURE`,
  `SUBGRAPH_CIRCUIT_BREAKER_REJECTED`, …), and
- a `graphql.error` event with `error.type`, `error.message`, and
  `hive.error.subgraph_name`.

Previously these subgraph-level spans looked "ok" even when the fetch
never produced a response, which was misleading in tracing UIs that
highlight failing spans. The error is now visible at the subgraph hop
where it actually originated.
