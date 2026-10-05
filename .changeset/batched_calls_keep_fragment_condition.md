---
hive-router: patch
node-addon: patch
---

# Fix batched calls running when their fragment's condition is false

Two calls to the same subgraph under the same conditional fragment can be batched into one. The batched call lost the fragment's condition, so it ran even when the condition was false.

```graphql
query ($a: Boolean!, $b: Boolean!) {
  users {
    reviews {
      product {
        upc
        ... on Product @include(if: $a) {
          notes # comes from subgraph `products`
          price @include(if: $b) # comes from subgraph `products`
        }
      }
    }
  }
}
```

Before, the call to `products` ran every time:

```
Flatten(path: "users.@.reviews.@.product|[Product]") {
  Fetch(service: "products") { ... on Product { ... on Product @include(if: $b) { price } ... on Product @include(if: $a) { notes } } }
}
```

After, it only runs when `$a` is true (condition is kept):

```
Include(if: $a) {
  Flatten(path: "users.@.reviews.@.product|[Product]") {
    Fetch(service: "products") { ... on Product { ... on Product @include(if: $b) { price } ... on Product @include(if: $a) { notes } } }
  }
}
```

`reviews` only sends the `__typename` that `products` needs when `$a` is true. With `$a: false`, the call went out without `__typename`, so `products` rejected the whole request, and every other field in that request came back `null`.
