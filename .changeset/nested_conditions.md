---
hive-router: patch
node-addon: patch
---

# Fix calls under nested conditions running when an outer condition is false

A call to another subgraph, planned under several conditional fragments or fields, only checked the innermost condition. With an outer condition false and the inner one true, the router could still call the subgraph, for fields the client had skipped.

```graphql
query ($a: Boolean!, $b: Boolean!) {
  userInA {
    id
    ... on User @include(if: $a) {
      ... on User @include(if: $b) {
        name # comes from subgraph `b`
      }
    }
  }
}
```

Before, the call to `b` only checked `$b`:

```
Include(if: $b) {
  Flatten(path: "userInA|[User]|[User]") { Fetch(service: "b") { ... on User { name } } }
}
```

After, it checks both:

```
Include(if: $a) {
  Include(if: $b) {
    Flatten(path: "userInA|[User]|[User]") { Fetch(service: "b") { ... on User { name } } }
  }
}
```
