---
hive-router: patch
node-addon: patch
---

# Fix several query planner bugs around `@requires`, `@provides` and `@interfaceObject`

- `@requires` after an entity hop no longer adds an extra fetch to the same subgraph ([#1539](https://github.com/graphql-hive/router/issues/1539)).
- Fields below `@provides` can now be resolved together with the subgraph's own fields, without extra `_entities` calls.
- Aliased `@requires` fields with different arguments (like `price(currency: USD)` and `price(currency: EUR)`) now get the right values after an entity hop.
