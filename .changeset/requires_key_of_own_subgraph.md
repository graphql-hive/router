---
hive-router: patch
---

# Send the key of the right subgraph to a `@requires` field

The entity call for a `@requires` field sent the key of whichever subgraph came first, not one of the subgraph that has the field. With `Item @key(fields: "group { id }")` in one subgraph and `Item @key(fields: "group { tag }")` in the one with the field, the call sent `group { id }`, so the subgraph couldn't find the item. It now sends that subgraph's own key. When the fields `@requires` reads have to come from another subgraph first, that can take an extra call.
