---
node-addon: minor
hive-router-query-planner: minor
hive-router: minor
hive-router-plan-executor: minor
---

# Introduce BatchFetch for compatible entity fetches to improve query performance

When multiple `Flatten(Fetch)` steps target the same subgraph and have compatible shape, the planner can group them into one batched fetch operation with aliases.

Batching keeps execution depth the same, but **reduces request fanout**.
In our benchmark query, **downstream requests drop from `13` to `7`** while the number of execution waves stays unchanged.
This should also reduce pressure on subgraphs, because entities are resolved in one batched subgraph call instead of being resolved across multiple incoming GraphQL requests, where the lack of DataLoader or another caching layer could otherwise cause duplicate resolution work.

Before: 

```graphql
Parallel {
  Flatten(path: "products.@") {
    Fetch(service: "inventory") {
      {
        ... on Product {
          upc
        }
      } =>
      {
        ... on Product {
          shippingEstimate
        }
      }
    }
  }
  Flatten(path: "topProducts.@") {
    Fetch(service: "inventory") {
      {
        ... on Product {
          upc
        }
      } =>
      {
        ... on Product {
          shippingEstimate
        }
      }
    }
  }
}
```

After:

```graphql
BatchFetch(service: "inventory") {
  {
    _e0 {
      paths: [
        "products.@"
        "topProducts.@"
      ]
      {
        ... on Product {
          upc
        }
      }
    }
  }
  {
    _e0: _entities(representations: $__batch_reps_0) {
      ... on Product {
        shippingEstimate
      }
    }
  }
}
```

When two entity fetches go to the same subgraph but request different output fields, they are batched into one `BatchFetch` node with two aliases, but share the same variables, to reduce the payload size.

```
BatchFetch(service: "inventory") {
  {
    _e0 {
      paths: [
        "products.@"
      ]
      {
        ... on Product {
          upc
        }
      }
    }
    _e1 {
      paths: [
        "products.@"
      ]
      {
        ... on Product {
          upc
        }
      }
    }
  }
  {
    _e0: _entities(representations: $__batch_reps_0) {
      ... on Product {
        shippingEstimate
      }
    }
    _e1: _entities(representations: $__batch_reps_0) {
      ... on Product {
        inStock
      }
    }
  }
}
```
