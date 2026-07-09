---
hive-router-plan-executor: minor
hive-router: patch
---

# Replace the schema state in the on_http_request plugin hook

Add `OnHttpRequestHookPayload::set_schema_state`, letting a plugin override the schema used for a request as early as `on_http_request`, and have it hold for the entire pipeline: parsing, validation, normalization, planning, execution, and introspection.

Previously, a plugin could only swap the schema at the validation stage (`on_graphql_validation` via `payload.with_schema(...)`), which left introspection (`__schema`/`__type`) unaffected, since it reads from the schema resolved earlier in the pipeline. Fields hidden from validation would still show up in introspection.

Plugins build and own their `Arc<SchemaState>` instances (e.g. via `SchemaState::from_supergraph_sdl` / `SchemaState::from_supergraph_document`, new public constructors on `hive_router::SchemaState`), each with its own fresh caches, so plan/validate/ normalize cache entries never leak across schema variants. The router does not manage the lifecycle of plugin-owned states: no background reload, no cache invalidation, no subscription force-close. Building a `SchemaState` is expensive (it builds a full query planner), so plugins should build one per schema variant up front, not per request.

Example:

```rust
fn on_http_request<'req>(
    &'req self,
    payload: OnHttpRequestHookPayload<'req>,
) -> OnHttpRequestHookResult<'req> {
    payload.set_schema_state(self.schema_state_for_request(&payload).clone());
    payload.proceed()
}
```

See the new [plugin_examples/replace_schema](https://github.com/graphql-hive/router/tree/main/plugin_examples/replace_schema) example for a full walkthrough.
