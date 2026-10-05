---
hive-router: patch
node-addon: patch
---

# Fix `MissingStep` when several fields need the same `@requires` chain

Queries asking for several fields that rely on the same chain of `@requires` fields, directly, through fragments or under aliases, could fail with `QUERY_PLAN_BUILD_FAILED`. The planner combines requests in several rounds, and a later round could still point at a request that an earlier round had already combined into another one.

```graphql
query {
  product {
    requestedA: canAffordWithAndWithoutDiscount
  }
  ... on Query {
    product {
      canAffordWithAndWithoutDiscount
    }
  }
  ... on Query {
    product {
      ... {
        ... on Product {
          requestedB: canAffordWithAndWithoutDiscount
        }
      }
      canAfford
    }
  }
}
```

Before: `failed to build fetch graph: MissingStep(...)`. After: the query plans correctly.
