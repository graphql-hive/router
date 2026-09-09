---
hive-router: patch
---

# Cached query plans no longer keep a parsed copy of every subgraph request

Each cached query plan held a parsed document for every request it sends to a subgraph, alongside
the request text. Execution only ever reads the text. The parsed copies existed so demand control
could compile its cost formula later, after the plan had been cached.

The cost formula is now compiled while the plan is being built and stored with the plan, so the
parsed documents can be dropped before the plan reaches the cache. A plan is either still being
planned or ready to execute, and only the first kind can hold parsed documents, so the ordering is
enforced by the compiler rather than by convention.

On the fixtures in this repository the stored plan node shrinks from 328 to 224 bytes, the stored
fetch from 320 to 216, and a stored subgraph operation from 144 to 40. The larger saving is the
parsed documents themselves, which are no longer retained at all.

## Fixes

- **A plugin that replaced a query plan could get the wrong cost limit.** An `OnQueryPlanEnd` hook
  returning a new plan kept the cost compiled for the old one. Replacement plans are now costed
  from their own contents, parsing each fetch back from its text. If that text cannot be parsed the
  request fails with `PLAN_COST_OPERATION_REBUILD_FAILED` rather than being treated as free.

Query plans returned by `hive-expose-query-plan` and sent to Hive Gateway are unchanged, byte for
byte. The rules for query costs are unchanged. There is nothing to configure.
