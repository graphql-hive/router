---
hive-router: patch
---

# Faster fetch graph building and batching in the query planner

- Find a fetch step to reuse by its subgraph, input type, response path and condition, instead of checking every child of the parent step.
- Compare response paths without type conditions without copying them.
- Check whether two fetch steps can be batched before the more expensive ancestor check.
