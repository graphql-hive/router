---
hive-router: patch
---

Fix graph construction failing for any supergraph where a `@provides` fieldset contains `__typename` (e.g. `@provides(fields: "item { __typename label }")`).

Fixes https://github.com/graphql-hive/router/issues/1455
