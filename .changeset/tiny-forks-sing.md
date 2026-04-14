---
hive-router-plan-executor: patch
---

Refactor executor response object handling to use `ValueObject` and map-style accessors across projection, merge, traversal, rewrites, and introspection paths. This centralizes key sorting and lookup behavior, reduces repeated binary-search boilerplate, and keeps runtime behavior consistent while improving maintainability.
