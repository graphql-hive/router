---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Avoid propagating @include/@skip conditions to unconditional fetches

Fixed query planner condition propagation logic to avoid wrapping unconditional fetches
in conditional blocks when merging steps. This ensures that fields without directives are
not incorrectly gated by conditions from other steps, allowing for correct execution of
queries with mixed conditional and unconditional selections.