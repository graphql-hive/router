---
hive-router: patch
---

# Run GraphQL parameters hooks for WebSocket operations

WebSocket `subscribe` payloads now run the `on_graphql_params` start hook and its registered end callbacks before parsing, validation, and execution. The start hook receives the synthetic WebSocket operation request and the already-decoded parameters in `OnGraphQLParamsStartHookPayload.graphql_params`.

When a hook ends preparation with an early response, the router sends the response's GraphQL body as a WebSocket `next` message followed by `complete`. HTTP-only response metadata, including its status and headers, cannot be represented by the GraphQL over WebSocket protocol and is not forwarded.
