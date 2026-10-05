---
hive-router: patch
node-addon: patch
---

# Fix wrong values for `@requires` fields with arguments

When `@requires` fields needed the same field with different arguments, like `price(currency: "GBP")` for `gbp` and `price(currency: "EUR")` for `eur`, the router fetched one of them under a temporary name.

It picked these names one request at a time, so a field could be fetched under one name and read under another. The subgraph then computed its field from the wrong value.

In this example:

```graphql
query {
  things {
    ... on Cat {
      gbp
    }
    eur
  }
}
```

Before, `pricing` computed `eur` for cats from `price`, which held the GBP price:

```graphql
# shop
... on Cat { price(currency: "GBP") _internal_qp_alias_0: price(currency: "EUR") }
# what pricing read
... on Cat { price }
```

After, it reads the EUR price:

```graphql
# what pricing reads
... on Cat { price: _internal_qp_alias_0 }
```

The router now picks these names once for the whole query, for each place in the response.
