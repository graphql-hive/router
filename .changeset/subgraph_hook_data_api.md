---
hive-router: minor
---

# `on_subgraph_execute` end hooks walk the subgraph's data

`OnSubgraphExecuteEndHookPayload` no longer has `execution_result: SubgraphResponse`. It has:

- `data`, a `SubgraphData`: what the subgraph sent for this fetch, before it's merged into the response. Plugins `walk` it like `OnExecuteEndHookPayload::data`, with the fetch's own response keys (`_entities` for an entity fetch) and fields typed by the objects' `__typename` and the schema: `set` a scalar or enum to a value of its type, `set_null` any value, or `set_null_with_error`.
- `errors`, the subgraph's GraphQL errors. These and the errors attached to nulls count as the subgraph's: their paths are mapped to the client's and error masking applies to them.
- `headers()`, `status()` and `extensions()`, read only: the subgraph's response headers, its HTTP status, and its `extensions` (read from the body each time `extensions()` is called). A hook that needs to change what the subgraph sent changes `data` and `errors`, or ends with its own response.

```rust
payload.on_end(|mut payload| {
    payload.data.walk(|value| {
        if value.is("User", "email") {
            value.set_null_with_error("email is private");
        }
    });
    payload.proceed()
})
```

The hooks now run in the execution pipeline once it has read the subgraph's body, so every subgraph response is read once, with or without plugins. Before, any plugin made the router read each response into a `Value` and write it back out.

A plugin that answers instead of the subgraph, from the start hook or the end hook, passes its response as JSON with `SubgraphResponse::from_json(body)` (errors it sets on that response come after the body's), or only errors with `SubgraphResponse::from_errors(errors)`; the router reads it like a subgraph's. `SubgraphResponse::data` and `extensions` are no longer public fields, so a response can't be built around the router's internal `Value`; `data()` still reads what an executor read. A response a plugin ends with replaces the subgraph's, and later end hooks don't run.
