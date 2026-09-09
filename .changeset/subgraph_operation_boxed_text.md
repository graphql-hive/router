---
hive-router: patch
---

# Subgraph request text is stored compactly and no longer re-rendered

A subgraph request is rendered once when the plan is built and never appended to, but it was stored
in a `String`, which keeps whatever capacity its growth last landed on. Across the fetches in this
repository's benchmark plan that was 1484 bytes held for 897 bytes of text - 39.6% of the storage
was slack. It is now stored as a boxed string, which keeps exactly the bytes. The stored subgraph
operation goes from 40 to 32 bytes, a stored fetch from 216 to 208.

## Fixes

- **Returning a query plan re-rendered every subgraph request from its parsed form** instead of
  serializing the text already stored. This happened on every `hive-expose-query-plan` response and
  every `node-addon` `plan()` call. The output is unchanged.
