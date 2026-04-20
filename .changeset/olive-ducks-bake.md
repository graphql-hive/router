---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
---

Fix query planner handling for combined `@skip` and `@include` conditions.

- Preserve both directives when converting inline fragment conditions into fetch step selections
- Build the expected nested condition nodes for combined skip/include execution paths
- Handle `SkipAndInclude` in selection matching, fetch-step rendering, and multi-type batch path hashing
- Add regression snapshot tests for field-level and fragment-level combined conditions

For example a query like this:

```graphql
query($skip: Boolean!, $include: Boolean!) {
  user {
    name @skip(if: $skip) @include(if: $include)
  }
}
```

Will now correctly generate a fetch step with an inline fragment that has both `@skip` and `@include` conditions, and the planner will properly evaluate the combined conditions when determining which selections to include in the execution plan.

- `@skip(if: $skip)` is true, the selection will be skipped regardless of the `@include` condition.
- `@include(if: $include)` is false, the selection will be skipped regardless of the `@skip` condition.
- Only if `@skip(if: $skip)` is false and `@include(if: $include)` is true, the selection will be included in the execution plan.