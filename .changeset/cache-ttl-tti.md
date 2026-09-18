---
hive-router: minor
---

# Configurable cache TTL and TTI

In-memory caches now support time-based expiry alongside `max_entries`:

```yaml
cache:
  router:
    parsing:
      max_entries: 1000
      time_to_live: 30m
      time_to_idle: 5m
  supergraph:
    validation:
      max_entries: 1000
      time_to_idle: 5m
    normalization:
      max_entries: 1000
      time_to_idle: 5m
    query_plans:
      max_entries: 1000
      time_to_idle: 5m
```

Both `time_to_live` and `time_to_idle` are optional and disabled by default, preserving the existing LRU-only behavior.

- `time_to_live` expires an entry after a fixed duration from its creation or last update, even if it is frequently accessed.
- `time_to_idle` expires an entry after it has not been read or updated for the configured duration.

When both are configured, the entry expires as soon as either condition is met.

Plugins can override each expiry setting independently for every supergraph variant.
Any expiry left unspecified inherits its value from the router configuration.
Overriding one setting does not affect the others.
To explicitly disable an expiry configured at the router level, set that expiry to `None`.

```rust
let mut options = SupergraphOptions::default();
options.cache.query_plans.set_time_to_idle(Some(Duration::from_secs(300)));
options.cache.query_plans.set_time_to_live(None);
```
