---
query-planner: patch
node-addon: patch
router: patch
---

# Prevent planner failure when combining conditional directives and interfaces

Fixed a bug where the query planner failed to handle the combination of conditional directives (`@include`/`@skip`) and the automatic `__typename` injection required for abstract types.
