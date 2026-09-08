---
hive-router: patch
---

# Cached query plans use much less memory

The router keeps planned queries in a cache. Each cached plan kept a parsed copy of every request it
sends to a subgraph, as well as the request text. The router only needs the text to run the query.
The parsed copies were kept only so demand control could calculate the query cost later.

The cost calculation is now stored with the plan and made while the plan is built. The parsed copies
can then be removed when the plan is cached. This also led to four smaller changes to plan storage.

Across 50 query plans, the total memory they use went down by **81.5%**, from 819,932 to 151,433
bytes. The largest plan in that set improved by 85.5%. At the same average size, a full cache of
1,000 plans would use about **2.9 MiB instead of 15.6 MiB**. These numbers do not include cache
bookkeeping, cost calculations, or extra memory kept by the memory manager.

Query plans returned by `hive-expose-query-plan` and sent to Hive Gateway are unchanged, byte for
byte. There is nothing to configure. The rules for query costs are also unchanged. The changes below
only make sure each cost is used with the right plan.

## Fixes

- **Query costs could come from the wrong plan when progressive `@override` was used.** The cost
  was stored under the query alone, but the plan also depended on the override context and plugin
  filters. Requests with different override labels could reuse the first cost that was made,
  including its cost split by subgraph. Costs now use the same key as their plan. Queries that only
  differ in introspection fields still share one cost.
- **A plugin that replaced a query plan could get the wrong cost limit.** An `OnQueryPlanEnd` hook
  that returned a new plan kept the cost for the old plan. New plans are now costed from their own
  contents. If a plan cannot be read again, the request fails with
  `PLAN_COST_OPERATION_REBUILD_FAILED` instead of being treated as free.
- **Returning a query plan rebuilt every subgraph request** instead of reusing the stored text. This
  happened for every exposed query plan and every `node-addon` `plan()` call.
- **The same paths in a batched entity fetch could be treated as different** when they had a type
  condition. The executor then repeated work it could have shared.
- **Error paths in a response were rebuilt at every step**, copying the full path and its text for
  each level of each item. This only happened when a subgraph returned an error.

## Removed

The `cost.formula_cache_hit` attribute is gone from operation spans. It was never recorded, and the
thing it described no longer exists. It was the same as the plan cache hit or miss already shown on
the span.
