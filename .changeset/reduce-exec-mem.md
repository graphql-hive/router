---
hive-router: patch
---

# Reduce memory usage at startup and during request execution

Reduce memory allocations and copying, lowering both idle memory usage and per-request overhead.

- Render the Laboratory page lazily, avoiding unnecessary memory usage when Laboratory is not accessed.
- Transfer response bodies to ntex without copying their buffers.
- Read subgraph HTTP responses incrementally to allow Hyper to reuse its read buffers.
- Reduce cloning during response merging by borrowing source values and moving entities into their final destination.
- Allocate JSON objects and arrays with exact capacities during deserialization.
- Avoid allocating type names during authorization metadata traversal.
- Reduce allocations in subgraph endpoint storage, request summaries, JSON logging, and request deduplication hashing.
