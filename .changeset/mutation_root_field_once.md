---
hive-router: patch
---

# Mutation and subscription fields run in one subgraph

A mutation field defined in more than one subgraph could run in each of them when the selected fields lived in different subgraphs, so its side effects happened more than once. The same went for a `@key` on the `Mutation` type, and for mutation fields inside a root fragment with `@include`/`@skip`. A shareable subscription field opened a stream in every subgraph. Each of these fields now runs in one subgraph, the one with the cheapest plan, and the rest comes through entity calls.
