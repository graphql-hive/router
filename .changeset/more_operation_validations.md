---
hive-console-sdk: patch
hive-router-query-planner: patch
graphql-tools: patch
hive-router-internal: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
hive-apollo-router-plugin: patch
---

# Improve GraphQL operation validation

- **Faster validation (2-3x):** rules now share a single `OperationVisitor` pass over the operation document instead of each rule visiting it independently.
- **New `UniqueInputFieldNames` rule:** input object fields are now kept as a list rather than a map, so duplicate fields are no longer silently deduplicated before validation. A query like `{ field(input: { value: 1, value: 2 }) }` is now correctly rejected.
- **Fixed `VariablesInAllowedPosition`:** now accounts for default values on variables, field arguments, and input object fields. Nullable variables used in a non-null argument that defines a default are no longer incorrectly rejected.
