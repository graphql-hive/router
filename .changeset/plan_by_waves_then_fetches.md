---
hive-router: patch
node-addon: patch
---

# Query plans with fewer subgraph requests, planned faster

The query planner now picks a plan by how many rounds of subgraph requests run one after another, then by how many requests it makes. When choosing how to get a field, it looks at what the plan already fetches, so fields that one request can serve together share it more often.

Compared with before:

- 142 plans make fewer subgraph requests in the same number of rounds.
- 13 plans need fewer rounds. 5 of those make one more request to save a round.
- 2 operations that failed with "Failed to find Satisfiable Key" now get a plan.
- 1 plan makes one more request in the same number of rounds.

Planning also takes 17–41% less time on the planner benchmarks, for example 4.57 s to 2.72 s and 634 ms to 378 ms on two large operations.
