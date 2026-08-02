---
hive-router-config: minor
hive-router-internal: patch
hive-router: minor
hive-router-plan-executor: patch
---

# Add `limits.max_request_header_size`

Adds a new `limits.max_request_header_size` configuration option (default: `64KiB`) that rejects requests whose HTTP headers exceed the configured size with `431 Request Header Fields Too Large`, before the request is processed.

Since the router propagates client headers (cookies, JWTs) to subgraphs, requests with oversized headers would previously be forwarded and rejected by the subgraph server's own header limit (e.g. Tomcat's 8KB default), surfacing as a confusing subgraph error. With this limit, such requests are rejected at the router with a clear error.
