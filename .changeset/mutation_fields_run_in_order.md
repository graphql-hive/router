---
hive-router: patch
node-addon: patch
---

# Ensure mutation run in sequence across subgraphs

The fields of a mutation have to run one after another. The router started the next field as soon as the previous field's own request was done, without waiting for the other requests that field needed, like an entity call to another subgraph.

```graphql
mutation {
  create: addProduct(input: { name: "new", price: 599.99 }) {
    # c
    isExpensive # comes from another subgraph
  }
  count: add(num: 1) #c
}
```

Before, `count` ran at the same time as the `isExpensive` call for `create`, because they originated from different subgraphs:

```
Sequence {
  Fetch(service: "a") { create: addProduct(...) }
  Parallel {
    Fetch(service: "c") { count: add(num: 1) }
    Flatten(path: "create") { Fetch(service: "b") { isExpensive } }
  }
}
```

After, `count` waits for it:

```
Sequence {
  Fetch(service: "a") { create: addProduct(...) }
  Flatten(path: "create") { Fetch(service: "b") { isExpensive } }
  Fetch(service: "c") { count: add(num: 1) }
}
```

Consecutive fields that go to the same subgraph are still sent together in one request, in order, if applicable.
