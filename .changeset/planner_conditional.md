---
hive-router-query-planner: patch
node-addon: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Fix planning for conditional inline fragments and field conditions

Fixed a query-planner bug where directive-only inline fragments (using `@include`/`@skip` without an explicit type condition) could fail during normalization/planning for deeply nested operations.

This update improves planner handling for conditional selections and adds regression tests to prevent these failures in the future.
