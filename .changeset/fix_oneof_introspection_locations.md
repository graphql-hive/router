---
hive-router: patch
---

Fix: `@oneOf` built-in directive declared with non-spec locations in introspection

The router's introspection response declared `@oneOf` as `on OBJECT | INTERFACE | UNION` instead of the spec-correct `on INPUT_OBJECT`.

Closes https://github.com/graphql-hive/router/issues/1479
