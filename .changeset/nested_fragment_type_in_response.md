---
hive-router: patch
node-addon: patch
---

# Fix `null` for a fragment inside a conditional fragment on an interface

A fragment on an object type inside a conditional fragment on an interface, like `... on Node @include(if: $y) { ... on Cat { eur } }`, lost its own type when the router put the response together. So a field resolved through an `@interfaceObject` came back as `null`.

```graphql
query ($y: Boolean!) {
  things {
    ... on Node @include(if: $y) {
      ... on Cat { eur }
    }
  }
}
```

With `$y: true`, before:

```json
{ "data": { "things": [{ "eur": null }, {}] } }
```

After:

```json
{ "data": { "things": [{ "eur": 1100 }, {}] } }
```
