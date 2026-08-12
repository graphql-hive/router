---
hive-router: patch
---

# Mask internal error details from client responses

Improve router's error handling by masking internal error details from client responses.

Client-caused errors still return their real message, since it only ever reflects the client's own request. Internal errors now always return a generic `"Internal server error"` message and never the underlying error message, which previously leaked details such as subgraph URLs, storage/network errors, and other backend internals. The real error is still logged for debugging purposes.

Error codes are unchanged. HTTP status codes are unchanged, except for GraphQL operation normalization and minification failures, which are now correctly treated as router-side bugs and always return `500` (previously `400`, or `200` based on `Accept` header) instead of being treated as a client mistake.
