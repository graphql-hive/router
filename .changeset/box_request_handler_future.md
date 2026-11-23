---
hive-router: patch
---

# Less CPU spent per request in the GraphQL endpoint

The request handler future is boxed, so it isn't copied around on every request. About 6.5 µs (~2%) less router CPU per request.
