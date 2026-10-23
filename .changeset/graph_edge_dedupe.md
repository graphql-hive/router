---
hive-router: patch
---

# Avoid duplicate graph edges and speed up supergraph decomposition

- A type that repeats `@join__type` for the same subgraph (once per `@key`) no longer adds duplicate edges to the query planner graph.
- Supergraph decomposition groups join types and enum values by subgraph in a single pass, instead of scanning every definition once per subgraph. This was slow for supergraphs with hundreds of subgraphs.
