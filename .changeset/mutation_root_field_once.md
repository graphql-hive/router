---
hive-router: patch
---

# Mutation and subscription fields run in one subgraph

A mutation field defined in more than one subgraph could run in each of them when the selected fields lived in different subgraphs, so its side effects happened more than once. The same went for a `@key` on the `Mutation` type, and for mutation fields inside a root fragment with `@include`/`@skip`. A shareable subscription field opened a stream in every subgraph. Each of these fields now runs in one subgraph, the one with the cheapest plan, and the rest comes through entity calls.

A mutation field is also done before the next one runs, entity calls included. A field in the same subgraph as the one before it used to go in the same request, with the earlier field's entity calls running after both, so they could read what the next field changed. Now the next field waits for them.
