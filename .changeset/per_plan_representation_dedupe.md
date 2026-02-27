---
hive-router-plan-executor: patch
hive-router: patch
---

Improve query execution performance by reusing subgraph entity responses within a single query plan execution. When the router needs the same representation fetch more than once, it now reuses the first result instead of sending duplicate subgraph requests.
