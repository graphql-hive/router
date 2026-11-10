---
hive-router: patch
---

# Query planner, projection and demand control fixes

- Entities inside lists of lists (`[[Product]]`, `[[[Product]]]`) are resolved. They used to come back as `null` without an error.
- An object inside a representation that misses a selected field is sent as `null` instead of without that field.
- A conditional fragment on a union field's own union type, like `... on Component @skip(if: $v)`, no longer fails with `NoPathsFound`.
- The same `@fromContext` field selected twice in one entity call shares one context variable, instead of sending one response key with two different arguments, which the subgraph rejects.
- Demand control applies `@include`/`@skip` on inline fragments when estimating cost.
- Demand control counts the fields of a conditional fragment on an interface or union, like `... on Product @include(if: $v)`, in the actual cost. They were left out.
- Demand control counts every object of a list of lists, like `[[Product]]`, and its fields in the actual cost, instead of one per inner list.
