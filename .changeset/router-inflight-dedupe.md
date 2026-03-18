---
hive-router: minor
hive-router-config: minor
hive-router-internal: minor
hive-router-plan-executor: minor
---

# Add router-level in-flight request deduplication for GraphQL queries

The router now supports deduplicating identical incoming GraphQL query requests while they are in flight, so concurrent duplicates can share one execution result.

## Configuration

A new router traffic-shaping section is available:

- `traffic_shaping.router.dedupe.enabled` (default: `true`)
- `traffic_shaping.router.dedupe.headers` as `all`, `none`, or `{ include: [...] }` (default: `all`)

Supported header config shapes:

```yaml
headers: all
```

```yaml
headers: none
```

```yaml
headers:
  include:
    - authorization
    - cookie
```

Header names are validated and normalized as standard HTTP header names.

## Deduplication key behavior

The router dedupe fingerprint includes:

- request method and path
- selected request headers (based on dedupe header policy)
- normalized operation hash
- GraphQL variables hash
- schema checksum
- GraphQL extensions

Schema checksum is derived from the loaded `SupergraphData` snapshot used for execution, preventing cross-schema sharing during schema reload transitions.
