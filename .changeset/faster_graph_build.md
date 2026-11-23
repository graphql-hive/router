---
hive-router: patch
---

# Faster query planner graph build

Building the query planner's graph from a supergraph is faster, which shortens router startup and supergraph reloads. On a supergraph with 745 subgraphs it went from 276 ms to 121 ms. The planner gets the same graph, so query plans don't change.
