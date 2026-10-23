---
hive-router: patch
---

# Speed up building the query planner graph

- Parse each `@key`, `@requires` and `@provides` selection once and share it between edges.
- Cache subgraph nodes while building the graph.
- Compute the possible types of each abstract type once and reuse them during normalization.
- Index which subgraphs resolve each root field once, instead of looking it up for every field.
- Skip types that can't have entity moves or outgoing edges, and handle enum leaves more cheaply.
