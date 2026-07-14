---
hive-router-plan-executor: major
hive-router: patch
---

# Drop `with_schema` from the `on_graphql_validation` plugin hook

Remove `OnGraphQLValidationStartHookPayload::with_schema`. Method was broken by design, it replaced the schema only for validation while parsing, introspection, normalization, planning, and execution continued using the request's original schema state. This could make a field disappear during validation while remaining visible through introspection, and it could leave schema-derived caches and planning state inconsistent with the schema used to validate the operation.

Plugins that need a request-specific schema should now build and retain a stable `Arc<Document>` for each schema variant and select it in `on_http_request`:

```rust
fn on_http_request<'req>(
    &'req self,
    payload: OnHttpRequestHookPayload<'req>,
) -> OnHttpRequestHookResult<'req> {
    payload.set_schema_document(self.document_for_request(&payload).clone());
    payload.proceed()
}
```

The router resolves the selected supergraph document into an internally owned `SchemaState` and reuses it for later requests that provide the same `Arc<Document>`. The selected schema then applies consistently to the entire request pipeline, including validation, introspection, normalization, planning, and execution.

Documents should be created when the plugin initializes or when the supergraph reloads, not per request. Creating a new `Arc<Document>` for every request defeats the router's schema-state cache and forces the schema state and query planner to be rebuilt.
