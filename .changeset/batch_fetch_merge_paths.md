---
hive-router: patch
---

# Faster batched entity fetches with many paths

When a batched entity fetch has to put entities in many places in the response, the router now walks all of those places in one pass, both when collecting the representations to send and when merging the entities back. Before, it walked each one from the root on its own. With 20 to 50 paths, collecting and merging are 1.5 to 4 times faster. The representations sent to subgraphs don't change.

Cached query plans also take less memory. A plan ready to run keeps its flatten and batch merge paths only as these walk structures, with one copy of every field name, so large plans take up to about a third less memory in the plan cache. The plan's JSON and its printed form don't change.

For plugins that read the query plan: a flatten node's `path` and a batch alias's `merge_paths` are now `TriePath` and `TriePaths` in a plan ready to run. `to_path()` and `to_paths()` give the paths.
