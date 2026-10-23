---
hive-router: patch
---

# Reduce temporary allocations during AST traversal

Avoid allocating strings for temporary names when hashing query-plan ASTs and rebuilding nullified operations.
