---
hive-router: patch
node-addon: patch
---

Allow `@deprecated` on arguments in introspection results

The router's built-in `@deprecated` definition now includes the `ARGUMENT_DEFINITION` location, as the GraphQL spec requires.
