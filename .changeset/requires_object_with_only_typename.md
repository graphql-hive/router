---
hive-router: patch
node-addon: patch
---

# Fix `@requires` dropping an object that only has `__typename`

When a subgraph needed a nested object through `@requires`, the router sent the object's `__typename` only together with another of its fields. An object of a type that none of the selection's fragments are for has nothing else to send, so the router dropped it: the field went missing, or the list lost the item.

```graphql
type Listing @key(fields: "id") {
  id: ID!
  pet: Animal @external
  rank: Float @requires(fields: "pet { __typename ... on Dog { tricks } ... on Cat { whiskers } }")
}
```

For a listing whose pet is a Bird, before, `ranking` got the listing without its pet:

```json
{ "__typename": "Listing", "id": "l3" }
```

After:

```json
{ "__typename": "Listing", "pet": { "__typename": "Bird" }, "id": "l3" }
```

In a list, like `pets { __typename ... on Dog { tricks } ... on Cat { whiskers } }`, a Bird is now kept in its place instead of being removed.

An object that misses a selected field is still left out, so an entity without its key isn't sent.
