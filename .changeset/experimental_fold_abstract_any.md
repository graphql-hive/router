---
hive-router-plan-executor: minor
hive-router-query-planner: minor
hive-router: minor
hive-router-config: minor
node-addon: patch
hive-router-internal: patch
---

## Add an experimental query planner option, `experimental_abstract_type_folding`

```yaml
query_planner:
    experimental_abstract_type_folding: true # false by default
```

Folds matching concrete object-type fragments in subgraph calls, into a shared interface fragment even when that interface is not the field's declared return type.

It's an opt-in addition to [`011be5b`](https://github.com/graphql-hive/router/commit/011be5bdbfb00bf1e415eb7a50e6be91f565ef05).

```diff
# queries `product-service` subgraph
query {
  products {
-    ... on Book  { id title }
-    ... on Movie { id title }
+    ... on Media { id title }
  }
}
```

The `products` field returns `Product` interface, but one object-type member of this interface called `Album` is not present in the query, therefore `... on Product {...}` is not possible to use (default behavior). With the feature flag enabled, both fragments are folded into `... on Media { ... }`, because `Book` and `Movie` are the only members of the `Media` interface in the `product-service` subgraph.
