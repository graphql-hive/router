---
hive-router: patch
---

# Query planner, projection and demand control fixes

- An abstract field selected again under `@include`/`@skip` keeps its fields with the condition off. The subgraph is now asked for the `__typename` the response needs to tell the types apart, not only under the condition.
- A field under a fragment on every type its parent can be, like `... on Query @include(if: $a) { feed { id } }` next to `feed { id }`, no longer goes missing from the response.
- The same `@fromContext` field selected twice in one entity call shares one context variable, instead of sending one response key with two different arguments, which the subgraph rejects.
- Demand control applies `@include`/`@skip` on inline fragments when estimating cost.
