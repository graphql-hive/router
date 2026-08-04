---
hive-router: patch
---

# Expose request correlation IDs to plugins and middlewares

Request/trace correlation IDs are now extracted and scoped by a dedicated middleware that wraps the entire request pipeline, instead of only inside the GraphQL handler. 

This means plugins, the coprocessor runtime, and any other middleware can now log with the same `request_id`/`trace_id` as the rest of the request.

Fixes https://github.com/graphql-hive/router/issues/1351
