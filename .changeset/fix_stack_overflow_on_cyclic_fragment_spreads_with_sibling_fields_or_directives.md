---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Fix stack overflow on cyclic fragment spreads with sibling fields or directives

A self-referential fragment that also selects a sibling field (`fragment A on Query { x ...A }`) or puts a directive on the cycling spread (`...A @include(if: $c)`) caused unbounded recursion during fragment inlining in normalization, overflowing the stack and crashing the process.
