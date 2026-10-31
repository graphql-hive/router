---
hive-router: patch
---

# Conditional `@requires` inputs stay with their condition

A `@requires` field under `@skip`/`@include` next to an unconditional field of the same entity, like `permissions @skip(if: $skip)` next to `basicPermissions`, no longer turns the unconditional field into `null` when the condition is off. Entity calls only merge when each side's input is there whenever the merged call runs.
