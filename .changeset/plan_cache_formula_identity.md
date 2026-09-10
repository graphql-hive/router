---
hive-router: patch
---

# Query costs could use the wrong plan with progressive `@override`

The demand control cost formula was cached using only the operation hash. The query plan uses a larger cache key. It also includes the override context and any operation changes made by plugins.
Because of this, two requests could use different query plans but still share the same cost formula. Whichever formula was created first was reused, including how the cost was split between subgraphs.

The cost formula is now stored together with the query plan it was created from. This makes sure a formula is only used with the correct plan. The separate formula cache has been removed.

Queries that differ only by introspection fields still share the same formula because they also share the same query plan.

## Removed

The `cost.formula_cache_hit` attribute has been removed from operation spans. It was never recorded, and the separate formula cache no longer exists. The existing plan cache hit attribute already provides this information.
