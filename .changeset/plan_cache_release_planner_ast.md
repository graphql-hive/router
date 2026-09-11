---
hive-router: patch
---

# Cached query plans no longer keep parsed subgraph operations

Cached query plans used to keep both the string and its parsed document for every subgraph fetch. Execution only needs the string
