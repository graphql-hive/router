---
hive-router-query-planner: patch
hive-router-plan-executor: patch
node-addon: patch
hive-router: patch
---

Fix fragments being dropped when multiple inline fragments target the same concrete type within an abstract type fragment.

Previously, when a query contained two or more inline fragments on the same concrete type nested inside an interface or union fragment, only the first fragment's fields were included in the query plan — all subsequent ones were silently dropped.

**Example query that previously returned only `title`:**

```graphql
query {
  films {
    ... on Node {
      ... on Film { title }
      ... on Film { director }
    }
  }
}
```

Both fields are now correctly returned.
