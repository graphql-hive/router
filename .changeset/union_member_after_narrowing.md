---
hive-router: patch
---

# Track a single union member after narrowing union paths

When several subgraphs can resolve a field that returns a union, the query planner narrows the candidate paths down to the members those subgraphs share. Each remaining path stands for one union member, but it used to keep the whole shared member set. So the planner could still follow fragments on the other shared members through it. Each path now keeps only its own member.
