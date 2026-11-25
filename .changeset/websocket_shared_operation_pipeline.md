---
hive-router: patch
---

# Refactor: WebSocket operations run the same pipeline as HTTP ones

Operations sent over WebSocket now go through the same code as operations sent over HTTP, from parsing to execution. That fixes three ways the WebSocket path had drifted:

- The coprocessor `graphql.request` stage now runs for WebSocket operations. Its header and body mutations apply, and a `break` ends the operation with the coprocessor's body, or with an error carrying the status when the break has no body.
- A plugin that ends an operation from `on_graphql_parse` or `on_graphql_validation` now sends its own response to WebSocket clients, instead of an "Unexpected early response" error.
- The request context now carries the operation's name and type, and the progressive override labels left to resolve, as it does for HTTP.

For custom builds that construct coprocessor stage inputs directly: `GraphqlRequestInput::new` now takes a `MutableRequestState` (method, URI and headers) instead of the HTTP request and a separate header map.
