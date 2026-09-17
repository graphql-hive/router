---
hive-router: minor
hive-router-macros: minor
---

# Configurable cache limits

In-memory cache limits are now configurable, either by entry count or by size:

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
      max_size: 256MB
```

`max_entries` and `max_size` are alternatives, not siblings - a cache is bounded one way or
the other, and setting both on the same cache is a config error.

`max_size` is written with a unit (`256MB`, `512MiB`, `64KiB`), read by the same parser as
`limits.max_request_body_size`. Note that `KB` means 1024 bytes there while `kB` and `MB` are
the SI 1000 and 1000000, so spell out `KiB`/`MiB`/`GiB` when you mean powers of two. A bare
number is rejected - give it a unit.

Under `max_size` every entry is weighed as it is stored, by walking the value that was just
built - so a query plan that fans out across many subgraphs is charged for its real shape
rather than for the length of the query that produced it. The weight covers the key, the
value, and a fixed allowance for the cache's own per-entry bookkeeping. A subtree shared
through an `Arc` is counted by each cache that points at it, so the total errs high rather
than low.

Each cache also reports what it is estimated to hold, next to its existing entry-count gauge:
`hive.router.parse_cache.size_bytes`, `validate_cache.size_bytes`,
`normalize_cache.size_bytes` and `plan_cache.size_bytes`. A cache bounded by `max_entries`
has no weigher, so its byte gauge stays at 0.

`cache.router` applies once per router process, while `cache.supergraph` applies to each
served supergraph variant.

Plugins creating additional supergraph variants can override their cache limits with
`SupergraphOptions::cache`:

```rust
use hive_router::config::primitives::byte_size::ByteSize;

let mut options = SupergraphOptions::default();
options.cache.query_plans.set_max_entries(100);
options.cache.validation.set_max_size(ByteSize::from_bytes(4 * 1024 * 1024));
```

Unset values inherit from `cache.supergraph` config or the default values.
