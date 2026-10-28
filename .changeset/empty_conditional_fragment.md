---
hive-router: patch
---

# Fix an empty conditional fragment in subgraph requests

Fixed an issue where the query planner could generate empty fragments when batching requests with `@include` or `@skip` directives, causing subgraphs to reject the query.
