---
hive-router: patch
node-addon: patch
---

# Fix `@requires` through a union losing its fields

When a `@requires` selected fields through a union, the router kept the union members of the other subgraphs instead of the current one. The required fields were dropped, and the subgraph got an invalid request.

For `price @requires(fields: "book { ... on Media { ... on Book { title } } }")` and this query:

```graphql
query {
  products {
    price
  }
}
```

Before, `catalog` was asked for `book` without any fields:

```graphql
{ products { __typename id book } }
```

After:

```graphql
{ products { __typename id book { title } } }
```
