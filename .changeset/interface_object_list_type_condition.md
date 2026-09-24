---
hive-router: patch
node-addon: patch
---

# Fix type conditions on a list of an `@interfaceObject`

Queries with a type condition on a list of an `@interfaceObject`, like `friends { ... on User { name } }`, no longer fail with `No paths found for selection item`.
