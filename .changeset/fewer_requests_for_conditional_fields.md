---
hive-router: patch
node-addon: patch
---

# Fewer subgraph requests for fields under `@include` and `@skip`

A call to another subgraph planned under `@include` or `@skip` always went out as a request of its own, so it could be skipped as a whole. Now it can go into a request to the same subgraph that is sent anyway: one for the same objects, or the one it gets its objects from. Its fields go under `... on T @include(if: $x)`, and the subgraph skips them when the condition is false. No variable value leads to more requests than before.

For example, in this operation:

```graphql
query ($includeRank: Boolean!) {
  cage {
    listings {
      rank @include(if: $includeRank)
    }
  }
}
```

Before, `search` was called twice: once for the listings, and once more, under `$includeRank`, for each listing's `pet`.

After, `pet` is part of the first call, and the condition goes inside the operation:

```graphql
... on Cage {
  listings {
    __typename
    id
    ... on Listing @include(if: $includeRank) { pet { __typename id } }
  }
}
```
