---
hive-router: patch
node-addon: patch
---

# Reduce memory retained by cached query and projection plans

Planner data is built using growable collections and then stored in caches, which can leave unused vector capacity allocated. The router now recursively shrinks data structures before caching them.

In tested queries, it resulted in a 50% reduction in retained heap memory.
