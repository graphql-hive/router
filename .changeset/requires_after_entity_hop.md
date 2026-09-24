---
hive-router: patch
node-addon: patch
---

# Fix extra fetches for `@requires` after an entity hop

- `@requires` after an entity hop no longer adds an extra fetch to the same subgraph ([#1539](https://github.com/graphql-hive/router/issues/1539)).
- Aliased `@requires` fields with different arguments (like `price(currency: USD)` and `price(currency: EUR)`) now get the right values after an entity hop.
