---
hive-router: patch
---

# Subgraph request deduplication works with more than one variable

When a subgraph request had two or more variables, their order changed from one request to the next. Two identical requests then looked different, so they were almost never deduplicated. Variables are now always sent in the same order (sorted by name), so identical requests share one subgraph call.
