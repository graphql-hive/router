---
hive-router: patch
node-addon: patch
---

# Fix `@include` dropped from a field after a conditional call to another subgraph

When a field under `@include(if: $x)` needed another subgraph, a later field with the same condition in the same request lost its `@include`.

The subgraph then resolved it even when `$x` was false. The same happened with a conditional fragment like `... on Query @include(if: $x)`.

```graphql
query ($withDetails: Boolean!, $term: String!) {
  record(id: "1") @include(if: $withDetails) {
    ... on User {
      id
      email
      invoices
    } # `invoices` comes from `billing`
  }
  listed: catalog {
    search(term: $term, kind: "item") @include(if: $withDetails) {
      ... on Item {
        id
        label
      }
    }
  }
}
```

Before, `accounts` got `search` without its condition:

```graphql
listed: catalog { search(kind: "item", term: $term) { ... } }
```

After:

```graphql
listed: catalog { search(kind: "item", term: $term) @include(if: $withDetails) { ... } }
```

Closes https://github.com/graphql-hive/router/issues/1647
