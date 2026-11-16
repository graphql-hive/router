---
hive-router: patch
---

Set client response headers from the `on_execute` end hook

`OnExecuteEndHookPayload` has `insert_response_header`, `append_response_header`, `get_response_header` and `remove_response_header` for the client response headers aggregated during execution. The headers are written after all end callbacks run, so plugin values win over the computed `cache-control`, except that mutations and responses with GraphQL errors (including errors added in end hooks) always get `no-store, no-cache, must-revalidate`, and they are part of the shared response, so requests deduplicated by inbound request deduplication receive them too even though they never run `on_execute`.

```rust
payload.on_end(|mut payload| {
    payload.insert_response_header(
        HeaderName::from_static("cache-control"),
        HeaderValue::from_static("no-store"),
    );
    payload.proceed()
})
```

Closes #1688
