---
hive-router: patch
---

# Add persisted documents support to WebSocket operations

WebSocket `subscribe` payloads can now omit the GraphQL query and provide a persisted document ID through their extensions. The router resolves the document before parsing, validation, and execution.

Persisted-document extraction, ID enforcement, resolution, metrics, and missing-ID logging use the supergraph selected for the WebSocket connection. This prevents an operation from resolving a document from one supergraph's manifest and executing it against another supergraph.
