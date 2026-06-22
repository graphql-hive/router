---
hive-router: patch
---

# Improve error handling when body limit reached

When rejecting a request whose `Content-Length` exceeds `max_request_body_size`, the router now drains a small bounded part of the request body before responding.

This lets the connection close cleanly so the client reliably receives the `413 Payload Too Large` response instead of a connection-reset/transport error.
