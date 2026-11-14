---
hive-router: patch
---

# Smaller cached query plans for queries with many fragments

The shape the router keeps for each fetch, to read subgraph responses fast, no longer repeats the fields of a fragment for every place that spreads it. Each key is stored once too. On our largest fixtures this makes a cached plan up to 23x smaller (19.8 MB to 0.86 MB) and builds it up to 18x faster. Queries that expanded past the shape's 16 MB limit and fell back to the slower parser now get a shape. Responses are read the same way and give the same results.
