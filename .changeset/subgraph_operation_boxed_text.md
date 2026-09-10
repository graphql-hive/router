---
hive-router: patch
---

# Subgraph request text is stored compactly and no longer re-rendered

The text of a subgraph request is built once when the plan is built, and never added to after
that. It was stored in a `String`, which can hold more memory than the text needs.

Across the fetches in this repository's benchmark plan, 1484 bytes were held for 897 bytes of
text. That means 39.6% of the memory was unused.

The text is now stored as a boxed string, which holds exactly the bytes it needs. A stored
subgraph operation goes from 40 to 32 bytes, and a stored fetch from 216 to 208.

## Fixes

- **Returning a query plan rebuilt the text of every subgraph request from its parsed form**
  instead of using the text that was already stored. This happened on every
  `hive-expose-query-plan` response and every `node-addon` `plan()` call. The output is unchanged.
