---
hive-router: patch
node-addon: patch
---

# Fix query planner failures around `@skip`, argument conflicts, nested `@requires` and entity interfaces

- An operation where `@skip`/`@include` drops every root field now responds with `{"data": {}}`, instead of failing with `QUERY_PLAN_BUILD_FAILED` ([#1189](https://github.com/graphql-hive/router/issues/1189)).
- When every root field of a fetch has the same `@skip`/`@include`, the whole fetch goes under a `Skip`/`Include` node, so no request is sent to the subgraph when it's skipped.
- The same field with different arguments under different type conditions no longer panics the worker ([#1311](https://github.com/graphql-hive/router/issues/1311)).
- A `@requires` nested inside another `@requires` whose fields come from more than one subgraph no longer fails with `NonSingleParent` ([#1310](https://github.com/graphql-hive/router/issues/1310)).
- A `@requires` reading fields of concrete types through an entity interface no longer fails with `No field found for name` or `UnexpectedMissingDefinition` ([#1309](https://github.com/graphql-hive/router/issues/1309), [#1308](https://github.com/graphql-hive/router/issues/1308)).
