---
hive-router: minor
---

# Warm up the plan cache before switching to a new supergraph

Every new supergraph starts with an empty plan cache, so the first requests after a schema publish wait on the query planner. With `cache.warmup` turned on, the router plans known operations against the new supergraph while the previous one keeps serving, and only switches over once that's done.

```yaml
cache:
  warmup:
    enabled: true # off by default
    timeout: 30s # switch over with whatever got planned by then
    hive: # optional, plans the slowest operations reported by Hive Console first
      endpoint: https://cdn.graphql-hive.com/artifacts/v1/<target>
      key: <cdn access token>
```

Operations come from, in this order:

- the slowest operations from Hive Console, when `hive` is set (`endpoint` and `key` fall back to `HIVE_CDN_ENDPOINT` and `HIVE_CDN_KEY`),
- every operation the previous supergraph served,
- plugins, through the new `on_cache_warmup` hook, which can add, drop or reorder the list.

Plans are built for requests with no progressive override labels. Requests with active labels, and operations that authorization or plugins trim, plan on their first request like before.

Two new metrics show how it went:

- `hive.router.plan_cache.warmup.operations_total`, by `source` (`hive_console`, `previous_supergraph`, `plugin`) and `result` (`warmed`, `failed`, `skipped`, `timed_out`)
- `hive.router.plan_cache.warmup.duration`, by `result` (`completed`, `timed_out`)
