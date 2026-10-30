---
hive-router: patch
---

# Query planner and representation fixes

- Entities inside lists of lists (`[[Product]]`, `[[[Product]]]`) are resolved. They used to come back as `null` without an error.
- A conditional fragment on a union field's own union type, like `... on Component @skip(if: $v)`, no longer fails with `NoPathsFound`.
- A conditional same-type fragment under an interface that's an `@interfaceObject` elsewhere no longer fails with `NoPathsFound`.
- Entity calls through an interface key only send the types the selection asks for, instead of every implementation.
- An object inside a representation that misses a selected field is sent as `null` instead of without that field.
