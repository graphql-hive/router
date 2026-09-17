---
hive-router: minor
---

# Configurable cache limits

In-memory cache limits are now configurable:

```yaml
cache:
  router:
    parsing:
      max_entries: 1000
  supergraph:
    validation:
      max_entries: 1000
    normalization:
      max_entries: 1000
    query_plans:
      max_entries: 1000
```

`cache.router` applies once per router process, while `cache.supergraph` applies to each served supergraph variant.

Plugins creating additional supergraph variants can override their cache limits with `SupergraphOptions::cache`:

```rust
let mut options = SupergraphOptions::default();
options.cache.query_plans.set_max_entries(100);
```

Unset values inherit from `cache.supergraph` config or the default values.
