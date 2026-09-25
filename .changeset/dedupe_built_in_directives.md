---
hive-router: patch
node-addon: patch
---

# Avoid duplicate built-in directive definitions in the consumer schema

Supergraphs that already define a built-in directive, like `@oneOf` (emitted by composition whenever a subgraph uses it), no longer end up with that directive defined twice in the consumer schema.
