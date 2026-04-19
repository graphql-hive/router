---
hive-router-plan-executor: patch
hive-router: patch
---

# Plugin System API improvements

Expose `EarlyHTTPResponse` instead of `PlanExecutionOutput` in the hooks that do not have internal fields like `response_headers_aggregator` etc, and it is easier to construct an HTTP response with a body, header map and status code.

```rust
payload.end_with_response(
    EarlyHTTPResponse {
        body,
        headers,
        status_code,
    }
);
```
