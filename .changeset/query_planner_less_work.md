---
hive-router: patch
---

# Faster query planning for large queries

Planning large queries does less work, with the same plans as before.

- Normalizing an operation copies less. On our largest test query, it makes 6.6M allocations instead of 9.1M and peaks at 212 MiB instead of 305 MiB.
- Optimizing the fetch graph skips checks that can't lead to a merge. On the same query, the fetch graph builds in 1.30 s instead of 1.96 s.
