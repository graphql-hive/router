---
hive-router: minor
---

# The `on_execute` start hook's `data` is JSON

`OnExecuteStartHookPayload::data` is now `Option<Bytes>`, a JSON object, instead of the router's internal `Value`. It's the introspection result for an operation with introspection fields, and `None` otherwise. A plugin can set it to give the plan data to start from:

```rust
payload.data = Some(Bytes::from_static(br#"{"me":{"id":"1"}}"#));
```

The router reads it with the operation's shape, and what the subgraphs send is merged into it, like before. Data that isn't a JSON object is left out, with a warning.

Introspection results are now written as JSON directly, and on the indexed pipeline every subscription event, from SSE, multipart, WebSocket or HTTP callback subgraphs, is read once, straight from its body.
