---
hive-router: patch
node-addon: patch
---

# Fix `@requires` nested inside another `@requires`

A `@requires` field inside the result of another `@requires` field failed to plan when the outer one needed fields from two different subgraphs.

With `checkup @requires(fields: "weight age")`, where `weight` and `age` come from two subgraphs, and `grade @requires(fields: "fee")` on `Checkup`:

```graphql
query {
  pet {
    checkup {
      grade
    }
  }
}
```

Before: `NonSingleParent(2)` (internal) error, so the router answered with `QUERY_PLAN_BUILD_FAILED`. The planner expected the request for `checkup` to wait for one other request, but it waits for two.

After: the query plans correctly.

Closes https://github.com/graphql-hive/router/issues/1310
