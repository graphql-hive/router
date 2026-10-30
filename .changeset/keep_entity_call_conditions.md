---
hive-router: patch
---

# Keep `@include`/`@skip` conditions on every entity call they apply to

The query planner could lose a condition on its way down to an entity call, so the call ran with the condition off and the subgraph resolved entities nobody asked for. This happened with:

- a fragment condition when a child has a condition of its own,
- nested conditions, when another branch fetches the same objects,
- a step merged with an unconditional sibling,
- batched calls under a type condition, for the types the path doesn't name.

A same-type fragment's condition now also moves to its inner fragments, so those entities are looked up once instead of twice.
