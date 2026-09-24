---
hive-router: patch
node-addon: patch
---

# Fix `@requires` on an object of the same type nested in an entity call

A `@requires` field on an object nested inside an entity of the same type, like `user { other { label } }` where both `user` and `other` are a `User`, no longer fails with `MissingPathInSelection`.
