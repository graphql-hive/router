---
hive-router-query-planner: minor
hive-router-plan-executor: patch
hive-router: patch
---

Improve query planning performance by reducing string allocation and cloning in the batch fetch optimization.

This change applies `FastStr` in internal optimizer paths used to build batched entity fetches, which significantly improves query-plan benchmark throughput while preserving planner behavior.
