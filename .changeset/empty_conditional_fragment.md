---
hive-router: patch
---

# Fix an empty conditional fragment in subgraph requests

When `@include` or `@skip` sat on one member of an abstract type and the fetches for the members got batched, the query planner could send `... on Magazine @include(if: $title) { }` to the subgraph. That's an empty selection set, so the subgraph rejected the request. It only showed up for some type names, because it depended on which type sorted first.
