---
hive-router: patch
---

# Match union type conditions when resolving requirements and contexts

The query planner now recognizes that a fragment on a union applies to any object type that belongs to that union, in addition to matching exact types and implemented interfaces. This prevents applicable fragment branches from being discarded when resolving requirements or contextual selections.

For example:

```graphql
union SearchResult = User | Product

type User {
  id: ID!
}

type Product {
  id: ID!
}
```

Given this selection:

```graphql
... on SearchResult {
  ... on User {
    id
  }
}
```

When the current type is `User`, the outer `SearchResult` condition now matches because `User` is a union member. Previously, that condition did not match.
