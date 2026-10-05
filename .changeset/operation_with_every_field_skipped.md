---
hive-router: patch
---

# Fix an error for an operation whose fields are all skipped

When `@skip(if: true)` or `@include(if: false)` removed every field of a query or a mutation, the router answered with an error instead of an empty result.

```graphql
query {
  topProducts @skip(if: true) {
    name
  }
}
```

Before: HTTP 500 with `QUERY_PLAN_BUILD_FAILED`, because there was nothing to plan.

After: HTTP 200 with `{ "data": {} }`.

Closes https://github.com/graphql-hive/router/issues/1189
