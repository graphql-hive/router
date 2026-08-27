---
hive-router: minor
---

Allow plugins to contribute a partition to the inbound (router-level) request dedupe key

Plugins can now call `add_inbound_dedupe_partition(u64)` to add custom partition to the router's inbound
query/subscription dedupe fingerprint. Requests with different partitions are never deduped
into the same in-flight response.

The method is only exposed on the `on_http_request` and `on_graphql_params` hook payloads, since
those are the only hooks that run before the router computes the fingerprint — calling it from a
later hook would silently have no effect, so it isn't offered there.

```rust
async fn on_graphql_params<'exec>(
    &'exec self,
    payload: OnGraphQLParamsStartHookPayload<'exec>,
) -> OnGraphQLParamsStartHookResult<'exec> {
    let partition = compute_partition_from_identity(&payload);
    payload.add_inbound_dedupe_partition(partition);
    payload.proceed()
}
```

This is useful when the built-in `traffic_shaping.router.dedupe.headers` allowlist is not enough
— for example, partitioning by an authenticated user extracted from a `Cookie` header without
hashing the full cookie data.

Closes https://github.com/graphql-hive/router/issues/1443
