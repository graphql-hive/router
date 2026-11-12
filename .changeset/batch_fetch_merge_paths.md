---
hive-router: patch
---

# Faster batched entity fetches with many paths

When a batched entity fetch has to put entities in many places in the response, the router now walks all of those places in one pass, both when collecting the representations to send and when merging the entities back. Before, it walked each one from the root on its own. With 20 to 50 paths, collecting and merging are 1.5 to 4 times faster. The representations sent to subgraphs don't change.
