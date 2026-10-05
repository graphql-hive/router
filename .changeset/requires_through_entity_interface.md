---
hive-router: patch
node-addon: patch
---

# Fix `@requires` that reads fields of an interface's types

A `@requires` reading fields of specific types behind an interface, like `pet { __typename ... on Dog { tricks } ... on Cat { whiskers } }` where `pet` is an `Animal`, failed with `QUERY_PLAN_BUILD_FAILED`.

The router looked up the cats and the dogs separately, then combined the two lookups and treated the result as covering every pet. But a pet can also be a `Bird`. So the combined lookup was then merged with the lookup for every `Animal`, which put `whiskers` and `tricks` on `Animal`, where they don't exist.

```graphql
query {
  listings {
    rank
  }
}
```

Before: `No field found for name 'whiskers' in type 'Animal'`, or `UnexpectedMissingDefinition("Animal")` with `@include` on `rank`. After: the query plans, and `catalog` gets each `pet` once.

Closes https://github.com/graphql-hive/router/issues/1308
Closes https://github.com/graphql-hive/router/issues/1309
