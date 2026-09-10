---
hive-router: patch
---

# Error paths in subgraph responses use fewer allocations

When a subgraph returns errors, the router builds a GraphQL error path while walking the response.

Previously, each step cloned the whole path built so far and then added one more segment. That meant repeated allocations as the path got deeper.

The router now reuses one path while walking the response. It pushes a segment when going deeper and pops it when going back. 
The path is only cloned when an error actually needs to keep it.
