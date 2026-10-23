---
hive-router: patch
---

# Fix planning of interface fields resolvable in several subgraphs

When a field returning an interface could be resolved by more than one subgraph, and its selection had more than one inline fragment, the query planner stopped using entity jumps for every field of that interface. If the selection also needed fields from other subgraphs, planning failed with `NoPathsFound`. Those fields can now be fetched from the subgraphs that provide them.
