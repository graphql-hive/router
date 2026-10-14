---
hive-router: minor
---

# Batch contextual entity fetches with configurable alias chunking

Entity fetches with different `@fromContext` argument values now combine distinct context-value groups into one subgraph request using aliased `_entities` fields, instead of sending one request per group. Each alias receives only its group's representations and contextual arguments. Responses and error paths are mapped back to the original client response shape.

The new `max_context_aliases_per_request` traffic-shaping option controls how many top-level aliases these requests may carry. It defaults to `15`, can be overridden per subgraph, and splits larger sets of context groups into additional requests rather than failing the operation.

```yaml
traffic_shaping:
  all:
    max_context_aliases_per_request: 15
  subgraphs:
    reviews:
      max_context_aliases_per_request: 5
```

For example, a regular entity fetch with 12 distinct context-value groups and a limit of `5` sends three requests containing 5, 5, and 2 groups. A single group keeps the original query unchanged. Batched fetches account for their existing aliases when choosing the number of groups per request, with at least one group per request.

Higher values reduce request overhead at the cost of larger query payloads. Lower values help accommodate subgraph alias or query-complexity limits. Set this option to `1` (or `0`) to disable context aliasing and send one request per context-value group. This is a chunk size, not a limit on the total number of context groups an operation may process.
