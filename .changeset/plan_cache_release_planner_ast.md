---
hive-router: patch
---

# Cached query plans no longer keep a parsed copy of every subgraph request

Each cached query plan held a parsed document for every request it sends to a subgraph, next to the
request text. Execution only ever reads the text. The parsed copies were kept so that demand
control could compile its cost formula later, after the plan had been cached.

The cost formula is now compiled while the plan is being built, and stored with the plan. This
means the parsed documents can be dropped before the plan reaches the cache.

A plan is either still being planned, or ready to execute. Only the first kind can hold parsed
documents, so the compiler enforces the order instead of a convention.

On the fixtures in this repository, a stored plan node goes from 328 to 224 bytes, a stored fetch
from 320 to 216, and a stored subgraph operation from 144 to 40. The larger saving is the parsed
documents themselves, which are no longer kept at all.

## Fixes

- **Cached plans no longer retain parsed subgraph documents.** Demand-control formulas are compiled
  while the plan is still being built, before it is converted into executable form, so cached plans
  retain the compiled formula without retaining the parsed documents.

Query plans returned by `hive-expose-query-plan` and sent to Hive Gateway are unchanged, byte for
byte. The rules for query costs are unchanged. There is nothing to configure.
