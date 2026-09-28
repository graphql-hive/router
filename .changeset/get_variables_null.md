---
hive-router: patch
---

# Accept `variables=null` in GET requests

A GET request with `variables=null` in the query string no longer fails with `400 Failed to parse GraphQL variables JSON`. Like `"variables": null` in a POST body, it now means the operation has no variables, so their default values apply.
