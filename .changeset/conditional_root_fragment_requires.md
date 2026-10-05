---
hive-router: patch
node-addon: patch
---

# Fix `@requires` fields under a conditional fragment on `Query`

A query with two or more `@requires` fields under a conditional fragment on the root type failed to plan.

```graphql
query ($x: Boolean!) {
  ... on Query @include(if: $x) {
    userInB {
      aName
      cName
    }
  }
}
```

Before: `MissingPathInSelection("|[Query] @include(if: $x).userInB", "Query")`. Moving the first `@requires` call into the root request removed the `... on Query @include(if: $x)` fragment that the second one needed.

After: the query plans correctly and does not error.
