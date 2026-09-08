---
hive-router: patch
---

# Faster response and entity projection with fewer allocations

Projecting subgraph responses into client responses, and projecting representations, is now faster and allocates less.

- Type-name resolution during projection is lazy and shared across sibling fields and list items instead of being cloned per node
- `__typename` lookups in `requires` projection and hashing happen at most once per object
- Repeated field lookups across list items reuse saved positions through a small stack-backed cache (heap-backed only for selections wider than 16 fields)

The `projection_lists`, `projection_nested_lists`, and `requires_loops` benchmarks show list projection running ~25-45% faster on typical selections, with no heap allocation for common short lists.
