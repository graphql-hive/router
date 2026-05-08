---
hive-router-query-planner: minor
node-addon: minor
hive-router-plan-executor: patch
hive-router: patch
---

# Fix conditional directive handling in response projection.

This fixes several edge cases where `@skip` and `@include` could produce an incorrect final response after query planning and projection planning.
