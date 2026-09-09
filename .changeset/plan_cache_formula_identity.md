---
hive-router: patch
---

# Query costs could come from the wrong plan when progressive `@override` was used

The demand control cost formula was cached under the operation hash alone. The query plan it
describes is cached under more than that: the override context and any plugin operation filtering
are part of the plan cache key too. Two requests that produced different plans could therefore share
one cost formula, and the first one compiled won - including its split of the cost across subgraphs.

The formula is now stored with the plan it was compiled from, in the same cache entry, so it can
only ever be used with that plan. The separate formula cache is gone. Queries that differ only in
introspection fields still share one formula, because they still share one plan.

## Removed

The `cost.formula_cache_hit` attribute is gone from operation spans. It was never recorded, and the
cache it described no longer exists; the plan cache hit already on the span covers it.
