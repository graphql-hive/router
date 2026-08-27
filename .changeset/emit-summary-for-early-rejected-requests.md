---
hive-router: patch
---

Emit a request summary log line for requests rejected early by a plugin's `on_http_request` hook (e.g. a malformed project key or missing authorization), not just requests that reach the GraphQL handler.

Previously, a plugin ending the request from `on_http_request` skipped the handler entirely, and the handler was the only place that emitted the summary log line - so no `router::request` line was ever printed for these requests, making them invisible to access logs.

Fixes https://github.com/graphql-hive/router/issues/1448
