---
hive-router: minor
hive-router-plan-executor: minor
---

# Propagate subgraph `extensions` to the client response

Subgraph GraphQL responses can carry an `extensions` object that can now be forwarded to the final client response, controlled by config.

Extension keys set by the router or plugins take precedence over subgraph-propagated values at all times.

`first`/`last` ordering matches processing order, which is deterministic for sequential plan nodes and non-deterministic for parallel fetches (same behaviour as response header propagation).

## Configuration

```yaml
extensions:
  propagate:
    algorithm: last # first | last | append. default: last
    allow: # optional key whitelist. omit to allow all keys
      - foo
      - bar
```

`algorithm` controls how the same key is merged when multiple subgraphs return it:

- `first` - keep the first subgraph's value, ignore later ones.
- `last` - overwrite with the last subgraph's value (default).
- `append` - collect every value into an array, always an array even for a single value.

`allow` is an optional whitelist of top-level extension keys. When omitted, all keys propagate.

The key `queryPlan` is always blocked regardless of config.

## Examples

Two subgraphs both return `extensions.foo`:

```
subgraph a: { "extensions": { "foo": { "some": ["array"] } } }
subgraph b: { "extensions": { "foo": { "some": "object" } } }
```

With `algorithm: first`:

```json
{ "extensions": { "foo": { "some": ["array"] } } }
```

With `algorithm: last`:

```json
{ "extensions": { "foo": { "some": "object" } } }
```

With `algorithm: append`:

```json
{ "extensions": { "foo": [{ "some": ["array"] }, { "some": "object" }] } }
```
