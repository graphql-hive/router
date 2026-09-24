---
hive-router: patch
node-addon: patch
---

# Use `@provides` fields for `@requires`

When the fields a `@requires` needs are made available by a `@provides` on the way, the field is now fetched together with them, from the same subgraph. Previously the planner ignored the provided fields and made extra `_entities` calls to get them again.
