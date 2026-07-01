---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Better detection of mutations in query-planner

When a mutation is encountered in an operation (e.g. `mutation { ... }`), the query planner needs to use `Sequence` instead of `Parallel` to ensure the mutation is executed in the correct order.

Previuosly, Hive Router was checking if `type Mutation` was used in the root step to determine if a mutation was present.

This change uses the actual incoming operation type (`mutation { ... }`) to determine if a mutation is present in a specific plan.
