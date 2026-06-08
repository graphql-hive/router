---
hive-router-plan-executor: patch
hive-router: patch
---

# Fix projection when only `__typename` is used as key

As described in [issue #1099](https://github.com/graphql-hive/router/issues/1099), when an entity's `@key` is only `__typename` (e.g. `@key(fields: "__typename")`), the executor built a correct query plan but never issued the `_entities` request to the other subgraph, leaving the cross-subgraph field resolved as `null`.

The representation projection skipped the `__typename` field and only emitted it alongside other fields, so a key using only `__typename` field produced an empty representation and the entity fetch was silently dropped.

The projection now emits a `{ "__typename": ... }` representation in this case, so the entity fetch runs and the field resolves as expected.
