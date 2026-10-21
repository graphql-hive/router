---
hive-router: minor
node-addon: patch
---

# Batch contextual entity fetches and cap aliases per subgraph request

Entity fetches with different `@fromContext` argument values now combine distinct context-value groups into one subgraph request using aliased `_entities` fields, instead of sending one request per group. Each alias receives only its group's representations and contextual arguments. Responses and error paths are mapped back to the original client response shape.

The new `max_aliases_per_request` traffic-shaping option caps how many aliased `_entities` fields one subgraph request may carry. It's not set by default, and can be set globally or per subgraph.

```yaml
traffic_shaping:
  all:
    max_aliases_per_request: 15
  subgraphs:
    reviews:
      max_aliases_per_request: 5
```

The cap applies to both places where the router adds aliases:

- **Query planning.** Compatible entity fetches to the same subgraph are batched into a `BatchFetch` with one alias per fetch shape. Without a cap batches are never split, same as before. With a cap, a batch with more aliases than the cap is split into several `BatchFetch` nodes that run in parallel. A leftover single fetch stays a regular fetch.
- **Contextual argument groups.** Without a cap, all groups of a fetch go in one request. With a cap, groups are split into additional requests rather than failing the operation. For example, a regular entity fetch with 12 distinct context-value groups and a cap of `5` sends three requests containing 5, 5, and 2 groups. A single group keeps the original query unchanged. In a batched fetch every group repeats all batch aliases, so each request carries `cap / batch aliases` groups, at least one.

Higher values reduce request overhead at the cost of larger query payloads. Lower values help accommodate subgraph alias or query-complexity limits, such as graphql-armor's max-aliases rule. Aliases from the client's own query are not counted. Set this option to `1` (or `0`) to send one `_entities` field per request.

Both limits are decided while planning: the planner splits batches and records how many context groups fit in one request on each fetch, so execution only splits the groups it finds in the data. Plugins that build `SupergraphOptions` themselves set the limit through the new `QueryPlannerOptions::max_aliases_per_request` (`Option<usize>`) and `subgraph_max_aliases_per_request` fields, which are filled from `traffic_shaping` when converting from the router config. The query planner's default is also no cap, so query plans produced by the node addon don't split batches either.
