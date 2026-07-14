---
hive-router-plan-executor: minor
hive-router: patch
---

# Select a schema document in the on_http_request plugin hook

Add `OnHttpRequestHookPayload::set_schema_document`, allowing a plugin to select a supergraph `Arc<Document>` during `on_http_request`. The selected schema applies to validation, normalization, planning, execution, and introspection for both HTTP and WebSocket requests.

The router resolves each document into an internally owned `SchemaState` with isolated schema-derived caches. Resolved states are kept in a strict FIFO cache with a maximum of 10 entries. The first request builds the state, later requests reuse it, and inserting an eleventh document evicts the oldest entry.

Plugins must retain and reuse the same `Arc<Document>` for each variant because cache keys use `Arc` allocation identity. Creating a new document or `Arc` per request causes a cache miss and an expensive planner rebuild.

If a selected document cannot be converted into a schema state, the router returns HTTP 500 rather than falling back to the default schema.

### Caveats for plugin authors

- Building a `SchemaState` includes a full query planner build. The first request for each document therefore has additional latency.
- Router-side supergraph reloads do not mutate or invalidate cached plugin-selected states and do not force-close subscriptions using them.
- `on_http_request` is synchronous. External feature or project lookups must be refreshed asynchronously in the plugin lifecycle so the hook only performs a synchronous lookup from request data to a stable `Arc<Document>`.

### Example

```rust
fn on_http_request<'req>(
    &'req self,
    payload: OnHttpRequestHookPayload<'req>,
) -> OnHttpRequestHookResult<'req> {
    if let Some(document) = self.schema_document_for_request(&payload) {
        payload.set_schema_document(document.clone());
    }
    payload.proceed()
}
```

See [`plugin_examples/replace_schema`](https://github.com/graphql-hive/router/tree/main/plugin_examples/replace_schema) for a complete example.
