---
hive-router: patch
---

# Faster hashing and path cloning in the query planner

- Use `ahash` for the query planner's hash maps and sets.
- Share merge path segments, so cloning a path is cheap.
- Borrow possible-type sets from the supergraph when merging batched fetch steps, instead of cloning them on every merge.
