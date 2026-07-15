---
hive-router-plan-executor: major
hive-router: patch
---

# Drop `with_schema` from the `on_graphql_validation` plugin hook

Remove `OnGraphQLValidationStartHookPayload::with_schema`. Replacing only the validation schema was unsafe because parsing, introspection, normalization, planning, demand control, execution, coprocessors, and schema-aware caches continued using the request's original supergraph.

Plugins that need a request-specific schema should construct and retain an `Arc<Supergraph>`, then select it in `on_http_request`:

```rust
fn on_http_request<'req>(
    &'req self,
    payload: OnHttpRequestHookPayload<'req>,
) -> OnHttpRequestHookResult<'req> {
    payload.set_supergraph(self.supergraph_for_request(&payload));
    payload.proceed()
}
```

Build each variant with `Supergraph::from_sdl` or `Supergraph::from_document` outside the request hot path and reuse the same `Arc<Supergraph>`. The router snapshots the selected supergraph and applies it to the complete request pipeline.

See `plugin_examples/replace_schema` for overriding a configured default and `plugin_examples/feature_flags` for plugin-only supergraph selection.
