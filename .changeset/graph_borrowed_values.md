---
hive-router: patch
---

# Borrow supergraph values instead of copying them

The query planner now borrows type names, field names and selections from the supergraph instead of owning copies of them. This means fewer allocations and lower memory use.
