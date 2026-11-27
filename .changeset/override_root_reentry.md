---
hive-router: patch
node-addon: patch
---

# `@override` is followed when a field returns a root type

A field that returns `Query` lets a query go back to the root in any subgraph, like `admin { scope { version } }`. When that field was moved to another subgraph with `@override`, the plan still asked the old subgraph for it, on its way back to `Query`. Now it asks the subgraph the field was moved to, and for a progressive `@override(label: ...)` the one the request goes to.
