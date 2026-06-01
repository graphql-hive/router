---
hive-router-plan-executor: minor
hive-router: minor
---

## Expose `context` and `request_context` on `on_graphql_error`

The `on_graphql_error` plugin hook now holds the `PluginContext` and a
`RequestContextPluginApi<OnGraphqlError>` as `context` and `request_context`, matching other request-scoped hooks (`on_http_request`, `on_execute`, etc.).

### Migration

`on_graphql_error` now has a generic over the request lifetime; signatures must be
updated from:

```rust
fn on_graphql_error(&self, mut payload: OnGraphQLErrorHookPayload) -> OnGraphQLErrorHookResult {
    // ...
}
```

to:

```rust
fn on_graphql_error<'req>(
    &'req self,
    mut payload: OnGraphQLErrorHookPayload<'req>,
) -> OnGraphQLErrorHookResult<'req> {
    // ...
}
```
