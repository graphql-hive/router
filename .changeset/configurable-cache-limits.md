---
hive-router: minor
---

# Configurable cache limits

The router's in-memory caches had hardcoded capacities. They can now be sized from the config file:

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

jwt:
  claims_cache_size: 10000
```

`cache.router` holds caches with a single instance per router process. `cache.supergraph` holds
caches with one instance per supergraph the router serves, so a plugin serving extra supergraph
variants multiplies that memory by the number of variants that are alive.

Setting a limit to `0` turns that cache off. All defaults match the previously hardcoded values, so
an existing config behaves exactly as before.

Plugins that build their own supergraph variants can override the per-supergraph limits for each
variant through `SupergraphOptions::cache`, for example to give a rarely used variant a smaller
query plan cache:

```rust
let mut options = SupergraphOptions::default();
options.cache.query_plans.set_max_entries(100);
```

Anything left unset inherits the router config's `cache.supergraph` value, so a plugin that does not
care about caches picks up whatever the operator configured.
