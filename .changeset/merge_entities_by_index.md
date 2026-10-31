---
hive-router: patch
---

# Faster entity merging

When entities come back from a subgraph, the router now finds where each one goes by its position in the request, instead of looking up the hash of its representation. Entities are also deduplicated with a faster hash, so hashing one entity takes 35–53% less time. Responses don't change.
