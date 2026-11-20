---
hive-router: patch
---

# Plan interface fields whose data lives in another subgraph

With `products: [Product]` returning an interface without a key, and `Product.dimensions: ProductDimension` returning an object without a key, a query asking for a `ProductDimension` field that the interface's subgraph doesn't have failed to plan with `NoPathsFound`. The planner now goes through the implementations (`... on Book`, `... on Magazine`), which have keys, and fetches the field from the subgraph that has it.
