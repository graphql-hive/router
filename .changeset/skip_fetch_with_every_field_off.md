---
hive-router: minor
node-addon: minor
---

# Don't send a fetch whose fields are all skipped

A fetch whose fields are all switched off by `@include`/`@skip` on different variables, like `details @include(if: $d)` and `inventory @include(if: $i)` with both off, is no longer sent ([#1733](https://github.com/graphql-hive/router/issues/1733)). It used to go out with nothing to resolve, and the subgraph still loaded each entity.

`FetchNode` in the query plan gets an optional `runIf`, a list of condition groups. The fetch runs when any of them holds, and always when it's missing. Each group has `includeIf` and `skipIf` variable names, and holds when every `includeIf` variable is `true` and no `skipIf` one is. Anything else that executes these plans has to check it too. Demand control only costs a fetch when it runs.

Such a fetch isn't batched with other entity calls for now.
