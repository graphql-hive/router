---
node-addon: patch
---

# Alias limits for `@fromContext` fetches

`new QueryPlanner(supergraphSdl, options)` takes optional planner options, the same ones the router reads from its config:

- `maxAliasesPerRequest`, like `traffic_shaping.all.max_aliases_per_request`
- `subgraphMaxAliasesPerRequest`, like `traffic_shaping.subgraphs.<name>.max_aliases_per_request`, by subgraph name
- `experimentalAbstractTypeFolding`, like `query_planner.experimental_abstract_type_folding`

`new QueryPlanner(supergraphSdl)` plans like before.

Every `Fetch` and `BatchFetch` with context variables now has `maxContextGroups` when there's a limit: the most groups of context values one request may carry. An executor puts up to that many groups in one request, each under its own aliases, and sends as many requests as it takes, the way the router does. `1` means one request per group, also when the fetch can't be aliased per group, so an executor no longer has to check that itself. Without it there is no limit and every group goes in one request.
