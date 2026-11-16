---
hive-router: patch
---

# Smaller cached query plans for queries with many fragments

Cached query plans for queries that reuse fragments are much smaller and faster to build. On our largest test query, a plan went from 19.8 MB to 0.78 MB and builds 24x faster. Very large queries that used to fall back to the slower response parser now use the fast one.
