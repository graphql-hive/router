---
hive-router: patch
---

# Treat `identity` subgraph `Content-Encoding` as uncompressed

When a subgraph responds with `Content-Encoding: identity` (the RFC 9110 token for "no encoding") or an empty `Content-Encoding` value, the router now passes the body through unchanged instead of failing the fetch with a `SUBGRAPH_RESPONSE_DECOMPRESSION_FAILURE` error. This matches how a missing `Content-Encoding` header is handled. Unrecognized compression algorithms still surface a clean decompression error.

Closes https://github.com/graphql-hive/router/issues/1511
